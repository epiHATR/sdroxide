//! Worker-thread wrapper around the `lorarx` PHY + mesh tracker.
//!
//! Same verbs as [`sdroxide_adsb::AdsbController`]: bounded IQ in (drop on
//! backpressure), unbounded control, non-blocking [`MeshtasticController::poll`].

use std::collections::HashMap;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{Receiver, Sender, bounded, select, unbounded};
use sdroxide_dsp::Complex32 as C32;
use sdroxide_types::{MeshtasticSettings, MeshtasticStatus};
use tracing::{info, warn};

use crate::lora::LoraRxPhy;
use crate::mesh::{AirframeError, CryptoKey, PacketHeader, decode_airframe};
use crate::track::Tracker;

const EMIT_INTERVAL: Duration = Duration::from_millis(500);
/// Same packet can arrive twice (direct + hop, or double TX). Drop repeats
/// keyed by Meshtastic `(from, packet_id)` for a short window — matches
/// OpenWebRX+ MeshtasticParser.
const DEDUP_TTL: Duration = Duration::from_secs(60);
const DEDUP_MAX: usize = 4096;

pub enum MeshtasticAction {
    Status(Box<MeshtasticStatus>),
}

struct Iq(Vec<C32>);

enum Ctl {
    Window { center_hz: f64, rate_hz: f64 },
    Config(MeshtasticSettings),
    ClearMessages,
    Stop,
}

pub struct MeshtasticController {
    iq_tx: Sender<Iq>,
    ctl_tx: Sender<Ctl>,
    res_rx: Receiver<MeshtasticAction>,
    worker: Option<JoinHandle<()>>,
}

impl MeshtasticController {
    pub fn new(window_center_hz: f64, window_rate_hz: f64, cfg: MeshtasticSettings) -> Self {
        let (iq_tx, iq_rx) = bounded::<Iq>(64);
        let (ctl_tx, ctl_rx) = unbounded::<Ctl>();
        let (res_tx, res_rx) = unbounded::<MeshtasticAction>();

        let worker = std::thread::Builder::new()
            .name("sdroxide-meshtastic".into())
            .spawn(move || {
                let mut w = Worker::new(window_center_hz, window_rate_hz, cfg);
                let mut last_emit = Instant::now();
                loop {
                    select! {
                        recv(ctl_rx) -> msg => match msg {
                            Ok(Ctl::Window { center_hz, rate_hz }) => {
                                w.set_window(center_hz, rate_hz);
                            }
                            Ok(Ctl::Config(next)) => w.set_config(next),
                            Ok(Ctl::ClearMessages) => w.tracker.clear_messages(),
                            Ok(Ctl::Stop) | Err(_) => break,
                        },
                        recv(iq_rx) -> msg => match msg {
                            Ok(Iq(iq)) => {
                                w.process(&iq);
                                if last_emit.elapsed() >= EMIT_INTERVAL {
                                    last_emit = Instant::now();
                                    if res_tx
                                        .send(MeshtasticAction::Status(Box::new(w.status())))
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                            }
                            Err(_) => break,
                        },
                    }
                }
            })
            .expect("spawn meshtastic worker");

        Self {
            iq_tx,
            ctl_tx,
            res_rx,
            worker: Some(worker),
        }
    }

    pub fn push_iq(&self, iq: &[C32]) {
        if iq.is_empty() {
            return;
        }
        let _ = self.iq_tx.try_send(Iq(iq.to_vec()));
    }

    /// Alias used by the engine's ADS-B-shaped call sites.
    pub fn on_rx_iq(&self, iq: &[C32]) {
        self.push_iq(iq);
    }

    pub fn set_window(&self, center_hz: f64, rate_hz: f64) {
        let _ = self.ctl_tx.send(Ctl::Window { center_hz, rate_hz });
    }

    pub fn set_config(&self, cfg: MeshtasticSettings) {
        let _ = self.ctl_tx.send(Ctl::Config(cfg));
    }

    pub fn clear_messages(&self) {
        let _ = self.ctl_tx.send(Ctl::ClearMessages);
    }

    pub fn poll(&self) -> Option<MeshtasticAction> {
        self.res_rx.try_recv().ok()
    }
}

impl Drop for MeshtasticController {
    fn drop(&mut self) {
        let _ = self.ctl_tx.send(Ctl::Stop);
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

struct Worker {
    #[allow(dead_code)]
    center_hz: f64,
    rate_hz: f64,
    cfg: MeshtasticSettings,
    key: CryptoKey,
    phy: LoraRxPhy,
    tracker: Tracker,
    listening: bool,
    /// `(from_node, packet_id)` → when first seen.
    seen: HashMap<(u32, u32), Instant>,
}

impl Worker {
    fn new(center_hz: f64, rate_hz: f64, cfg: MeshtasticSettings) -> Self {
        let cfg = cfg.sane();
        let key = CryptoKey::from_settings(&cfg);
        let tracker = Tracker::new(cfg.region);
        let phy = LoraRxPhy::new(rate_hz);
        let w = Self {
            center_hz,
            rate_hz,
            cfg,
            key,
            phy,
            tracker,
            listening: true,
            seen: HashMap::new(),
        };
        w.log_settings("worker-start");
        w
    }

    fn is_duplicate(&mut self, from: u32, packet_id: u32) -> bool {
        let now = Instant::now();
        let key = (from, packet_id);
        if let Some(t) = self.seen.get(&key)
            && now.duration_since(*t) < DEDUP_TTL
        {
            return true;
        }
        self.seen.insert(key, now);
        if self.seen.len() > DEDUP_MAX {
            self.seen.retain(|_, t| now.duration_since(*t) < DEDUP_TTL);
        }
        false
    }

    fn log_settings(&self, why: &str) {
        let psk = match self.cfg.psk_kind {
            sdroxide_types::MeshtasticPskKind::Default => "default".to_string(),
            sdroxide_types::MeshtasticPskKind::Simple => {
                format!("simple{}", self.cfg.psk_simple)
            }
            sdroxide_types::MeshtasticPskKind::Key16 => "aes128-hex".into(),
            sdroxide_types::MeshtasticPskKind::Key32 => "aes256-hex".into(),
        };
        info!(
            why,
            region = %self.cfg.region.label(),
            center_mhz = self.center_hz / 1e6,
            rate_hz = self.rate_hz,
            psk = %psk,
            lorarx = self.phy.unavailable().unwrap_or("ok"),
            "meshtastic worker settings"
        );
    }

    fn set_window(&mut self, center_hz: f64, rate_hz: f64) {
        let moved = (center_hz - self.center_hz).abs() >= 1.0
            || (rate_hz - self.rate_hz).abs() >= 1.0;
        self.center_hz = center_hz;
        if (rate_hz - self.rate_hz).abs() >= 1.0 {
            self.rate_hz = rate_hz;
            self.phy.set_rate(rate_hz);
        }
        if moved {
            self.log_settings("window");
        }
    }

    fn set_config(&mut self, cfg: MeshtasticSettings) {
        let cfg = cfg.sane();
        self.tracker.set_region(cfg.region);
        self.key = CryptoKey::from_settings(&cfg);
        self.cfg = cfg;
        self.log_settings("config");
    }

    fn process(&mut self, iq: &[C32]) {
        let packets = self.phy.push(iq);
        let unix = now_unix();
        for pkt in packets {
            match decode_airframe(&pkt.bytes, &self.key, Some(pkt.snr_db), pkt.rssi_dbm, unix)
            {
                Ok(dec) => {
                    if self.is_duplicate(dec.header.from, dec.header.id) {
                        continue;
                    }
                    let preview = match &dec.update {
                        crate::track::TrackerUpdate::Text { text, .. } => {
                            text.chars().take(48).collect::<String>()
                        }
                        other => format!("{other:?}")
                            .chars()
                            .take(48)
                            .collect::<String>(),
                    };
                    info!(
                        from = %PacketHeader::format_node(dec.header.from),
                        ch = dec.header.channel,
                        snr_db = ?pkt.snr_db,
                        crc_ok = ?pkt.crc_ok,
                        bytes = pkt.bytes.len(),
                        preview = %preview,
                        "meshtastic packet decoded"
                    );
                    self.tracker.apply(dec.update);
                }
                Err(AirframeError::Truncated { bytes }) => {
                    warn!(
                        bytes,
                        snr_db = ?pkt.snr_db,
                        crc_ok = ?pkt.crc_ok,
                        "meshtastic PHY bytes but shorter than mesh header"
                    );
                }
                Err(AirframeError::BadPayload { header, want_ch }) => {
                    let ch_match = header.channel == want_ch[0] || header.channel == want_ch[1];
                    warn!(
                        from = %PacketHeader::format_node(header.from),
                        to = %PacketHeader::format_node(header.to),
                        id = format!("{:08x}", header.id),
                        ch = header.channel,
                        want_ch = ?want_ch,
                        ch_match,
                        broadcast = header.is_broadcast(),
                        snr_db = ?pkt.snr_db,
                        crc_ok = ?pkt.crc_ok,
                        bytes = pkt.bytes.len(),
                        "meshtastic PHY bytes but mesh decode failed (PSK/bit errors?)"
                    );
                }
            }
        }
    }

    fn status(&self) -> MeshtasticStatus {
        let unavailable = self.phy.unavailable().map(str::to_string);
        let listening = self.listening && unavailable.is_none();
        self.tracker.status(listening, unavailable)
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
