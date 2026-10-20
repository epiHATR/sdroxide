//! MQTT decode publisher: push digi / WSPR / sonde / mesh / ADS-B JSON to a broker.
//!
//! A dedicated thread runs rumqttc's connection loop; the engine only
//! `try_send`s typed jobs. JSON encoding and broker I/O happen off the radio
//! thread so a slow or dead broker never stalls decode.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TrySendError};
use rumqttc::{Client, MqttOptions, QoS, Transport};
use sdroxide_types::{AdsbAircraft, Decode, MeshtasticMessage, MqttConfig, Rs41Sonde, WsprSpot};
use serde::Serialize;
use tracing::{info, warn};

const MAX_PENDING: usize = 256;
/// rumqttc request-channel capacity. Oversized so a brief broker stall does
/// not immediately force drops; overflow still drops via [`Client::try_publish`].
const MQTT_REQUEST_CAP: usize = 256;

#[derive(Debug)]
enum Job {
    Digi { topics: Vec<String>, body: DigiPayload },
    Json { topic: String, body: JsonBody },
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum JsonBody {
    Wspr(WsprPayload),
    Sonde(SondePayload),
    Mesh(MeshPayload),
    Adsb(AdsbPayload),
}

/// Running MQTT publisher, or a no-op stub when disabled.
pub struct MqttPublisher {
    cfg: MqttConfig,
    rx_call: String,
    rx_grid: String,
    tx: Option<Sender<Job>>,
    /// Cloned into shutdown so we can wake `connection.iter` / `recv_timeout`.
    client: Option<Client>,
    stop: Option<Arc<AtomicBool>>,
    join: Option<JoinHandle<()>>,
}

impl Default for MqttPublisher {
    fn default() -> Self {
        Self {
            cfg: MqttConfig::default(),
            rx_call: String::new(),
            rx_grid: String::new(),
            tx: None,
            client: None,
            stop: None,
            join: None,
        }
    }
}

impl Drop for MqttPublisher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl MqttPublisher {
    /// Update the call/grid stamped on every payload. Cheap; call whenever
    /// station identity or the radio's report grid changes.
    pub fn set_identity(&mut self, rx_call: &str, rx_grid: &str) {
        self.rx_call = rx_call.to_string();
        self.rx_grid = rx_grid.to_string();
    }

    pub fn apply(&mut self, cfg: MqttConfig, rx_call: &str, rx_grid: &str) {
        self.set_identity(rx_call, rx_grid);
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
            if call.is_empty() { "sdroxide".into() } else { format!("sdroxide-{call}") }
        } else {
            cfg.client_id.trim().to_string()
        };
        let (client, connection) = {
            let mut opts = MqttOptions::new(&client_id, cfg.host.trim(), cfg.port);
            opts.set_keep_alive(Duration::from_secs(30));
            if !cfg.user.is_empty() {
                opts.set_credentials(&cfg.user, &cfg.password);
            }
            if cfg.tls {
                opts.set_transport(Transport::tls_with_default_config());
            }
            Client::new(opts, MQTT_REQUEST_CAP)
        };
        let client_worker = client.clone();
        let join = std::thread::Builder::new()
            .name("sdroxide-mqtt".into())
            .spawn(move || worker(cfg, client_worker, connection, rx, stop_t))
            .expect("spawn mqtt worker");
        self.tx = Some(tx);
        self.client = Some(client);
        self.stop = Some(stop);
        self.join = Some(join);
        info!(host = %self.cfg.host, port = self.cfg.port, "mqtt publisher started");
    }

    fn shutdown(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::SeqCst);
        }
        // Wake the event loop so `recv_timeout` / reconnect wait returns.
        if let Some(client) = self.client.take() {
            let _ = client.try_disconnect();
        }
        self.tx = None;
        if let Some(j) = self.join.take() {
            // Never block the radio thread on a stuck broker — detach after a
            // short wait; the stop flag + disconnect already asked it to exit.
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            loop {
                if j.is_finished() {
                    let _ = j.join();
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    warn!("mqtt worker still stopping — detaching");
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    pub fn publish_digi(&self, d: &Decode, mode: &str, dial_hz: f64) {
        let Some(tx) = &self.tx else { return };
        let mut topics = Vec::with_capacity(2);
        if let Some(t) = self.cfg.digi.trimmed() {
            topics.push(t.to_string());
        }
        if mode.eq_ignore_ascii_case("FT8")
            && let Some(t) = self.cfg.ft8.trimmed()
        {
            topics.push(t.to_string());
        }
        if topics.is_empty() {
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
            rx_call: self.rx_call.clone(),
            rx_grid: self.rx_grid.clone(),
        };
        self.try_enqueue(tx, Job::Digi { topics, body });
    }

    pub fn publish_wspr(&self, s: &WsprSpot) {
        let Some(topic) = self.cfg.wspr.trimmed() else { return };
        let Some(tx) = &self.tx else { return };
        if s.is_heard_by_other() {
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
            rx_call: self.rx_call.clone(),
            rx_grid: self.rx_grid.clone(),
        };
        self.try_enqueue(tx, Job::Json { topic: topic.to_string(), body: JsonBody::Wspr(body) });
    }

    pub fn publish_sonde(&self, s: &Rs41Sonde) {
        let Some(topic) = self.cfg.sonde.trimmed() else { return };
        let Some(tx) = &self.tx else { return };
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
            rx_call: self.rx_call.clone(),
            rx_grid: self.rx_grid.clone(),
        };
        self.try_enqueue(tx, Job::Json { topic: topic.to_string(), body: JsonBody::Sonde(body) });
    }

    pub fn publish_mesh(&self, m: &MeshtasticMessage) {
        let Some(topic) = self.cfg.mesh.trimmed() else { return };
        let Some(tx) = &self.tx else { return };
        let body = MeshPayload {
            ts: m.unix,
            from: m.from.clone(),
            to: m.to.clone(),
            text: m.text.clone(),
            raw: m.raw.clone(),
            snr_db: m.snr_db,
            rx_call: self.rx_call.clone(),
            rx_grid: self.rx_grid.clone(),
        };
        self.try_enqueue(tx, Job::Json { topic: topic.to_string(), body: JsonBody::Mesh(body) });
    }

    /// One aircraft row as compact JSON. Omits track history to keep payloads small.
    pub fn publish_adsb(&self, a: &AdsbAircraft) {
        let Some(topic) = self.cfg.adsb.trimmed() else { return };
        let Some(tx) = &self.tx else { return };
        let body = AdsbPayload {
            ts: a.last_at,
            icao: a.hex(),
            callsign: a.callsign.clone(),
            category: a.category.clone(),
            lat: a.lat,
            lon: a.lon,
            altitude_ft: a.altitude_ft,
            gnss_altitude_ft: a.gnss_altitude_ft,
            ground_speed_kt: a.ground_speed_kt,
            track_deg: a.track_deg,
            vertical_rate_fpm: a.vertical_rate_fpm,
            on_ground: a.on_ground,
            squawk: a.squawk,
            emergency: a.emergency.clone(),
            rssi_dbfs: a.rssi_dbfs,
            frames: a.frames,
            source: a.source.label().to_string(),
            first_at: a.first_at,
            last_at: a.last_at,
            last_pos_at: a.last_pos_at,
            rx_call: self.rx_call.clone(),
            rx_grid: self.rx_grid.clone(),
        };
        self.try_enqueue(tx, Job::Json { topic: topic.to_string(), body: JsonBody::Adsb(body) });
    }

    fn try_enqueue(&self, tx: &Sender<Job>, job: Job) {
        match tx.try_send(job) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                warn!("mqtt publish queue full — dropping");
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

fn worker(
    cfg: MqttConfig,
    client: Client,
    mut connection: rumqttc::Connection,
    rx: Receiver<Job>,
    stop: Arc<AtomicBool>,
) {
    let qos = if cfg.qos >= 1 { QoS::AtLeastOnce } else { QoS::AtMostOnce };
    let client_pub = client.clone();
    let stop_pub = Arc::clone(&stop);
    let pub_thread = std::thread::Builder::new()
        .name("sdroxide-mqtt-pub".into())
        .spawn(move || {
            while !stop_pub.load(Ordering::SeqCst) {
                match rx.recv_timeout(Duration::from_millis(200)) {
                    Ok(job) => publish_job(&client_pub, qos, job),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .ok();

    // `recv_timeout` (not `iter`) so the stop flag is honoured promptly —
    // otherwise shutdown waits up to keep-alive for the next notification.
    while !stop.load(Ordering::SeqCst) {
        match connection.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok(_event)) => {}
            Ok(Err(e)) => {
                warn!(error = %e, "mqtt connection error");
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
            Err(rumqttc::RecvTimeoutError::Timeout) => {}
            Err(rumqttc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = client.try_disconnect();
    drop(client);
    if let Some(j) = pub_thread {
        let _ = j.join();
    }
    info!("mqtt publisher stopped");
}

fn publish_job(client: &Client, qos: QoS, job: Job) {
    match job {
        Job::Digi { topics, body } => {
            let payload = match serde_json::to_vec(&body) {
                Ok(p) => p,
                Err(e) => {
                    warn!(error = %e, "mqtt json encode failed");
                    return;
                }
            };
            for topic in topics {
                try_publish(client, &topic, qos, &payload);
            }
        }
        Job::Json { topic, body } => {
            let payload = match serde_json::to_vec(&body) {
                Ok(p) => p,
                Err(e) => {
                    warn!(error = %e, "mqtt json encode failed");
                    return;
                }
            };
            try_publish(client, &topic, qos, &payload);
        }
    }
}

fn try_publish(client: &Client, topic: &str, qos: QoS, payload: &[u8]) {
    // Never use blocking `publish`: rumqttc's request channel applies
    // backpressure and can stall this thread when the broker is slow.
    if let Err(e) = client.try_publish(topic, qos, false, payload) {
        warn!(error = %e, topic, "mqtt publish dropped");
    }
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
struct AdsbPayload {
    ts: i64,
    icao: String,
    callsign: String,
    category: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    altitude_ft: Option<i32>,
    gnss_altitude_ft: Option<i32>,
    ground_speed_kt: Option<f32>,
    track_deg: Option<f32>,
    vertical_rate_fpm: Option<i32>,
    on_ground: bool,
    squawk: Option<u16>,
    emergency: Option<String>,
    rssi_dbfs: f32,
    frames: u32,
    source: String,
    first_at: i64,
    last_at: i64,
    last_pos_at: i64,
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

    #[test]
    fn adsb_payload_omits_track() {
        let mut a = AdsbAircraft::new(0x3C6444, 1_000);
        a.callsign = "DLH123".into();
        a.lat = Some(48.1);
        a.lon = Some(11.5);
        a.track.push((48.1, 11.5));
        let body = AdsbPayload {
            ts: a.last_at,
            icao: a.hex(),
            callsign: a.callsign.clone(),
            category: None,
            lat: a.lat,
            lon: a.lon,
            altitude_ft: None,
            gnss_altitude_ft: None,
            ground_speed_kt: None,
            track_deg: None,
            vertical_rate_fpm: None,
            on_ground: false,
            squawk: None,
            emergency: None,
            rssi_dbfs: a.rssi_dbfs,
            frames: a.frames,
            source: a.source.label().to_string(),
            first_at: a.first_at,
            last_at: a.last_at,
            last_pos_at: a.last_pos_at,
            rx_call: "XV4HT".into(),
            rx_grid: "OL20".into(),
        };
        let v: serde_json::Value = serde_json::to_value(&body).unwrap();
        assert_eq!(v["icao"], "3C6444");
        assert_eq!(v["callsign"], "DLH123");
        assert!(v.get("track").is_none());
    }
}
