//! MQTT decode publisher: push digi / WSPR / sonde / mesh JSON to a broker.
//!
//! A dedicated thread runs rumqttc's connection loop; the engine enqueues
//! publish jobs on a bounded channel so a dead broker never stalls decode.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TrySendError};
use rumqttc::{Client, MqttOptions, QoS, Transport};
use sdroxide_types::{Decode, MeshtasticMessage, MqttConfig, Rs41Sonde, WsprSpot};
use serde::Serialize;
use tracing::{info, warn};

const MAX_PENDING: usize = 256;

#[derive(Debug, Clone)]
struct Job {
    topic: String,
    payload: Vec<u8>,
}

/// Running MQTT publisher, or a no-op stub when disabled.
pub struct MqttPublisher {
    cfg: MqttConfig,
    tx: Option<Sender<Job>>,
    stop: Option<Arc<AtomicBool>>,
    join: Option<JoinHandle<()>>,
}

impl Default for MqttPublisher {
    fn default() -> Self {
        Self { cfg: MqttConfig::default(), tx: None, stop: None, join: None }
    }
}

impl Drop for MqttPublisher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl MqttPublisher {
    pub fn apply(&mut self, cfg: MqttConfig, rx_call: &str) {
        // Topic-only edits must not bounce the broker connection.
        let reconnect = self.cfg.enabled != cfg.enabled
            || self.cfg.host != cfg.host
            || self.cfg.port != cfg.port
            || self.cfg.tls != cfg.tls
            || self.cfg.user != cfg.user
            || self.cfg.password != cfg.password
            || self.cfg.client_id != cfg.client_id
            || self.cfg.qos != cfg.qos
            || self.tx.is_some() != cfg.broker_configured();
        self.cfg = cfg.clone();
        if !reconnect {
            return;
        }
        self.shutdown();
        if !cfg.broker_configured() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(MAX_PENDING);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = Arc::clone(&stop);
        let client_id = if cfg.client_id.trim().is_empty() {
            let call = rx_call.trim();
            if call.is_empty() {
                "sdroxide".into()
            } else {
                format!("sdroxide-{call}")
            }
        } else {
            cfg.client_id.trim().to_string()
        };
        let join = std::thread::Builder::new()
            .name("sdroxide-mqtt".into())
            .spawn(move || worker(cfg, client_id, rx, stop_t))
            .expect("spawn mqtt worker");
        self.tx = Some(tx);
        self.stop = Some(stop);
        self.join = Some(join);
        info!(host = %self.cfg.host, port = self.cfg.port, "mqtt publisher started");
    }

    fn shutdown(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::SeqCst);
        }
        self.tx = None;
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }

    pub fn publish_digi(
        &self,
        d: &Decode,
        mode: &str,
        dial_hz: f64,
        rx_call: &str,
        rx_grid: &str,
    ) {
        let Some(topic) = self.cfg.digi.trimmed() else { return };
        if self.tx.is_none() {
            return;
        }
        let body = DigiPayload {
            ts: d.slot_utc,
            mode: mode.to_string(),
            dial_hz,
            snr_db: d.snr_db,
            dt: d.dt,
            audio_hz: d.audio_hz,
            message: d.message.clone(),
            from: d.from.clone(),
            to: d.to.clone(),
            grid: d.grid.clone(),
            is_cq: d.is_cq,
            rx_call: rx_call.to_string(),
            rx_grid: rx_grid.to_string(),
        };
        self.enqueue(topic, &body);
    }

    pub fn publish_wspr(&self, s: &WsprSpot, rx_call: &str, rx_grid: &str) {
        let Some(topic) = self.cfg.wspr.trimmed() else { return };
        if self.tx.is_none() || s.is_heard_by_other() {
            return;
        }
        let body = WsprPayload {
            ts: s.slot_utc,
            call: s.call.clone(),
            grid: s.grid.clone(),
            snr_db: s.snr_db,
            dt: s.dt,
            drift: s.drift_hz,
            freq_hz: s.freq_hz,
            power_dbm: s.power_dbm,
            rx_call: rx_call.to_string(),
            rx_grid: rx_grid.to_string(),
        };
        self.enqueue(topic, &body);
    }

    pub fn publish_sonde(&self, s: &Rs41Sonde, rx_call: &str, rx_grid: &str) {
        let Some(topic) = self.cfg.sonde.trimmed() else { return };
        if self.tx.is_none() {
            return;
        }
        let body = SondePayload {
            ts: s.last_at,
            serial: s.serial.clone(),
            type_: s.kind.label().to_string(),
            subtype: s.subtype.clone(),
            lat: s.lat,
            lon: s.lon,
            alt_m: s.altitude_m,
            freq_hz: s.frequency_hz,
            temp_c: s.temperature_c,
            humidity_pct: s.humidity_pct,
            rx_call: rx_call.to_string(),
            rx_grid: rx_grid.to_string(),
        };
        self.enqueue(topic, &body);
    }

    pub fn publish_mesh(&self, m: &MeshtasticMessage, rx_call: &str, rx_grid: &str) {
        let Some(topic) = self.cfg.mesh.trimmed() else { return };
        if self.tx.is_none() {
            return;
        }
        let body = MeshPayload {
            ts: m.unix,
            from: m.from.clone(),
            to: m.to.clone(),
            text: m.text.clone(),
            raw: m.raw.clone(),
            snr_db: m.snr_db,
            rx_call: rx_call.to_string(),
            rx_grid: rx_grid.to_string(),
        };
        self.enqueue(topic, &body);
    }

    fn enqueue<T: Serialize>(&self, topic: &str, body: &T) {
        let Some(tx) = &self.tx else { return };
        let payload = match serde_json::to_vec(body) {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "mqtt json encode failed");
                return;
            }
        };
        let job = Job { topic: topic.to_string(), payload };
        match tx.try_send(job) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                warn!("mqtt publish queue full — dropping");
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

fn worker(cfg: MqttConfig, client_id: String, rx: Receiver<Job>, stop: Arc<AtomicBool>) {
    let mut opts = MqttOptions::new(client_id, cfg.host.trim(), cfg.port);
    opts.set_keep_alive(Duration::from_secs(30));
    if !cfg.user.is_empty() {
        opts.set_credentials(&cfg.user, &cfg.password);
    }
    if cfg.tls {
        opts.set_transport(Transport::tls_with_default_config());
    }

    let (client, mut connection) = Client::new(opts, 32);

    let qos = if cfg.qos >= 1 { QoS::AtLeastOnce } else { QoS::AtMostOnce };
    let client_pub = client.clone();
    let stop_pub = Arc::clone(&stop);
    let pub_thread = std::thread::Builder::new()
        .name("sdroxide-mqtt-pub".into())
        .spawn(move || {
            while !stop_pub.load(Ordering::SeqCst) {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(job) => {
                        if let Err(e) =
                            client_pub.publish(&job.topic, qos, false, job.payload.as_slice())
                        {
                            warn!(error = %e, topic = %job.topic, "mqtt publish failed");
                        }
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .ok();

    for notification in connection.iter() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if let Err(e) = notification {
            warn!(error = %e, "mqtt connection error");
            // Brief pause then rumqttc will reconnect via the iterator.
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    drop(client);
    if let Some(j) = pub_thread {
        let _ = j.join();
    }
    info!("mqtt publisher stopped");
}

#[derive(Serialize)]
struct DigiPayload {
    ts: i64,
    mode: String,
    dial_hz: f64,
    snr_db: i16,
    dt: f32,
    audio_hz: f32,
    message: String,
    from: Option<String>,
    to: Option<String>,
    grid: Option<String>,
    is_cq: bool,
    rx_call: String,
    rx_grid: String,
}

#[derive(Serialize)]
struct WsprPayload {
    ts: i64,
    call: String,
    grid: Option<String>,
    snr_db: i16,
    dt: f32,
    drift: f32,
    freq_hz: f64,
    power_dbm: i16,
    rx_call: String,
    rx_grid: String,
}

#[derive(Serialize)]
struct SondePayload {
    ts: i64,
    serial: String,
    #[serde(rename = "type")]
    type_: String,
    subtype: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    alt_m: Option<f32>,
    freq_hz: f64,
    temp_c: Option<f32>,
    humidity_pct: Option<f32>,
    rx_call: String,
    rx_grid: String,
}

#[derive(Serialize)]
struct MeshPayload {
    ts: i64,
    from: String,
    to: String,
    text: String,
    raw: String,
    snr_db: Option<f32>,
    rx_call: String,
    rx_grid: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdroxide_types::SondeKind;

    #[test]
    fn digi_json_round_trip_shape() {
        let d = Decode {
            slot_utc: 1_760_784_000,
            snr_db: -12,
            dt: 0.2,
            audio_hz: 1234.5,
            message: "CQ K1ABC FN42".into(),
            to: None,
            from: Some("K1ABC".into()),
            grid: Some("FN42".into()),
            is_cq: true,
            cq_to: None,
            free_text: false,
            rr73_to: None,
        };
        let body = DigiPayload {
            ts: d.slot_utc,
            mode: "FT8".into(),
            dial_hz: 14_074_000.0,
            snr_db: d.snr_db,
            dt: d.dt,
            audio_hz: d.audio_hz,
            message: d.message.clone(),
            from: d.from.clone(),
            to: d.to.clone(),
            grid: d.grid.clone(),
            is_cq: d.is_cq,
            rx_call: "XV4HT".into(),
            rx_grid: "OL20".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(v["from"], "K1ABC");
        assert_eq!(v["mode"], "FT8");
    }

    #[test]
    fn sonde_type_field_renames() {
        let s = Rs41Sonde::new("V1234567".into(), 403e6, SondeKind::Rs41, 100);
        let body = SondePayload {
            ts: s.last_at,
            serial: s.serial,
            type_: "RS41".into(),
            subtype: None,
            lat: Some(48.2),
            lon: Some(16.3),
            alt_m: Some(12_000.0),
            freq_hz: s.frequency_hz,
            temp_c: None,
            humidity_pct: None,
            rx_call: "XV4HT".into(),
            rx_grid: "OL20".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(v["type"], "RS41");
        assert!(v.get("type_").is_none());
    }
}
