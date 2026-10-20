//! Mode-S Beast binary TCP output — dump1090 / readsb compatible.
//!
//! Clients connect to the listen port (default 30005) and receive framed
//! Mode-S messages. The demod worker only `try_send`s accepted frames; this
//! module owns encode, accept, and write on its own thread so a slow peer never
//! stalls decode.
//!
//! Wire format (Binary Beast / AVR.BEAST):
//! `0x1a` type timestamp[6] signal payload — type `'2'` short (7 bytes) or
//! `'3'` long (14 bytes). Any `0x1a` in the body is escaped as `0x1a 0x1a`.

use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use sdroxide_types::AdsbBeastConfig;
use tracing::{debug, info, warn};

/// How many accepted frames may wait for the Beast thread before drops.
const FRAME_QUEUE: usize = 1024;
/// Acceptor / write loop wake interval.
const POLL: Duration = Duration::from_millis(20);
/// Give a slow client this long per write before we drop it.
const WRITE_TIMEOUT: Duration = Duration::from_millis(50);

/// One accepted Mode-S frame for Beast encoding.
#[derive(Debug, Clone)]
pub struct BeastFrame {
    /// 7 or 14 Mode-S bytes (already CRC-accepted).
    pub bytes: Vec<u8>,
    /// Last accepted RSSI, dBFS (negative).
    pub rssi_dbfs: f32,
    /// Sample index on the demod stream clock when the frame was accepted.
    pub sample_index: u64,
    /// Demod sample rate (Hz) — used to convert `sample_index` to 12 MHz ticks.
    pub rate_hz: f64,
}

/// Engine-facing handle: start/stop the listener and hand a frame sink to the
/// ADS-B worker.
pub struct BeastServer {
    frame_tx: Sender<BeastFrame>,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    addr: String,
}

impl BeastServer {
    /// Bind and start serving. Bind failure surfaces here so Settings can show it.
    pub fn start(cfg: &AdsbBeastConfig) -> Result<Self, String> {
        let want = cfg.addr();
        let listener = TcpListener::bind(&want).map_err(|e| format!("bind {want}: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| format!("{want}: {e}"))?;
        let addr = listener.local_addr().map(|a| a.to_string()).unwrap_or(want);

        let (frame_tx, frame_rx) = bounded::<BeastFrame>(FRAME_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_t = Arc::clone(&stop);
        let join = std::thread::Builder::new()
            .name("sdroxide-adsb-beast".into())
            .spawn(move || serve(listener, frame_rx, stop_t))
            .map_err(|e| format!("spawn beast thread: {e}"))?;

        info!(%addr, "ADS-B Beast TCP listening");
        Ok(BeastServer { frame_tx, stop, join: Some(join), addr })
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// Clone for the ADS-B worker. `try_send` only — never blocks demod.
    pub fn frame_tx(&self) -> Sender<BeastFrame> {
        self.frame_tx.clone()
    }

    /// Non-blocking enqueue from the demod path. Full queue → drop + debug.
    pub fn try_push(tx: &Sender<BeastFrame>, frame: BeastFrame) {
        match tx.try_send(frame) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                debug!("ADS-B Beast queue full — dropping frame");
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

impl Drop for BeastServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Dropping the sender wakes `recv_timeout` when the queue drains.
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
        info!("ADS-B Beast TCP stopped");
    }
}

fn serve(listener: TcpListener, rx: Receiver<BeastFrame>, stop: Arc<AtomicBool>) {
    let mut clients: Vec<TcpStream> = Vec::new();
    let mut buf = Vec::with_capacity(64);

    while !stop.load(Ordering::Relaxed) {
        // Accept newcomers.
        loop {
            match listener.accept() {
                Ok((stream, peer)) => {
                    if let Err(e) = stream.set_nodelay(true) {
                        debug!(%peer, error = %e, "beast set_nodelay");
                    }
                    if let Err(e) = stream.set_write_timeout(Some(WRITE_TIMEOUT)) {
                        debug!(%peer, error = %e, "beast set_write_timeout");
                    }
                    info!(%peer, "ADS-B Beast client connected");
                    clients.push(stream);
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) => {
                    warn!(error = %e, "ADS-B Beast accept");
                    break;
                }
            }
        }

        // Drain frames (or wait briefly so accept keeps pace).
        let mut drained = 0usize;
        loop {
            let frame = if drained == 0 {
                match rx.recv_timeout(POLL) {
                    Ok(f) => f,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match rx.try_recv() {
                    Ok(f) => f,
                    Err(_) => break,
                }
            };
            drained += 1;
            buf.clear();
            encode_beast(&frame, &mut buf);
            clients.retain_mut(|c| match c.write_all(&buf) {
                Ok(()) => true,
                Err(e) => {
                    debug!(error = %e, "ADS-B Beast client write — dropping");
                    false
                }
            });
            if stop.load(Ordering::Relaxed) {
                return;
            }
        }
    }
}

/// Encode one Beast binary message into `out` (cleared by caller).
pub fn encode_beast(frame: &BeastFrame, out: &mut Vec<u8>) {
    let len = frame.bytes.len();
    let typ = match len {
        7 => b'2',
        14 => b'3',
        _ => return,
    };
    out.push(0x1a);
    out.push(typ);
    let ticks = mlat_ticks(frame.sample_index, frame.rate_hz);
    let mut ts = [0u8; 6];
    for (i, b) in ts.iter_mut().enumerate() {
        *b = ((ticks >> (40 - 8 * i)) & 0xff) as u8;
    }
    escape_push(out, &ts);
    escape_push(out, &[signal_level(frame.rssi_dbfs)]);
    escape_push(out, &frame.bytes);
}

fn escape_push(out: &mut Vec<u8>, bytes: &[u8]) {
    for &b in bytes {
        out.push(b);
        if b == 0x1a {
            out.push(0x1a);
        }
    }
}

/// dump1090-style 12 MHz MLAT clock from the demod sample index.
fn mlat_ticks(sample_index: u64, rate_hz: f64) -> u64 {
    let rate = rate_hz.max(1.0);
    ((sample_index as f64) * (12_000_000.0 / rate)) as u64
}

/// Map dBFS (−100…0) into Beast's 1…255 signal byte.
fn signal_level(rssi_dbfs: f32) -> u8 {
    let v = ((rssi_dbfs + 100.0) / 100.0 * 255.0).clamp(1.0, 255.0);
    v as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_frame_type_and_length() {
        let f = BeastFrame {
            bytes: vec![0x02; 7],
            rssi_dbfs: -40.0,
            sample_index: 0,
            rate_hz: 2_400_000.0,
        };
        let mut out = Vec::new();
        encode_beast(&f, &mut out);
        assert_eq!(out[0], 0x1a);
        assert_eq!(out[1], b'2');
        // 0x1a + type + 6 ts + 1 sig + 7 payload (no escapes) = 16
        assert_eq!(out.len(), 16);
    }

    #[test]
    fn long_frame_type() {
        let f = BeastFrame {
            bytes: vec![0x8d; 14],
            rssi_dbfs: -20.0,
            sample_index: 12_000,
            rate_hz: 2_400_000.0,
        };
        let mut out = Vec::new();
        encode_beast(&f, &mut out);
        assert_eq!(out[1], b'3');
        assert_eq!(out.len(), 23);
    }

    #[test]
    fn escapes_0x1a_in_payload() {
        let mut bytes = vec![0u8; 7];
        bytes[3] = 0x1a;
        let f = BeastFrame { bytes, rssi_dbfs: -50.0, sample_index: 0, rate_hz: 2_400_000.0 };
        let mut out = Vec::new();
        encode_beast(&f, &mut out);
        // One extra escape byte.
        assert_eq!(out.len(), 17);
        // Payload starts after 0x1a type ts[6] sig = index 9.
        assert_eq!(out[9 + 3], 0x1a);
        assert_eq!(out[9 + 4], 0x1a);
    }

    #[test]
    fn mlat_scales_with_sample_rate() {
        // One second of samples at 2.4 Msps → 12e6 ticks.
        let ticks = mlat_ticks(2_400_000, 2_400_000.0);
        assert_eq!(ticks, 12_000_000);
    }
}
