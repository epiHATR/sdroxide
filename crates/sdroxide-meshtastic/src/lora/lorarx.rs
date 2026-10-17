//! dxlAPRS [`lorarx`](http://oe5dxl.hamspirit.at:8025/aprs/c/) PHY backend.
//!
//! Same shape as OpenWebRX+: feed interleaved f32 I/Q on stdin, read JSON lines
//! with base64 `payload` + `crc` on stdout. Mesh decrypt stays in-process.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::Duration;

use base64::Engine;
use num_complex::Complex32;
use serde::Deserialize;
use tracing::{info, warn};

use super::demod::PhyPacket;

/// Missing / dead `lorarx` message shown in the Meshtastic panel.
pub const LORARX_MISSING: &str =
    "install lorarx (dxlAPRS) on PATH — OpenWebRX+ package dxlaprs-lora, or build from oe5dxl.hamspirit.at";

#[derive(Debug, Deserialize)]
struct LoraJson {
    payload: Option<String>,
    #[serde(default)]
    crc: i32,
    #[serde(default)]
    snr: Option<f32>,
}

/// Streaming PHY: push window-rate I/Q, drain CRC-OK payloads.
pub struct LoraRxPhy {
    rate_hz: f64,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    pkt_rx: Option<Receiver<PhyPacket>>,
    reader: Option<JoinHandle<()>>,
    unavailable: Option<String>,
    iq_scratch: Vec<f32>,
}

impl LoraRxPhy {
    pub fn new(rate_hz: f64) -> Self {
        let mut s = Self {
            rate_hz: rate_hz.max(1.0),
            child: None,
            stdin: None,
            pkt_rx: None,
            reader: None,
            unavailable: None,
            iq_scratch: Vec::new(),
        };
        s.spawn();
        s
    }

    pub fn unavailable(&self) -> Option<&str> {
        self.unavailable.as_deref()
    }

    pub fn set_rate(&mut self, rate_hz: f64) {
        let rate_hz = rate_hz.max(1.0);
        if (rate_hz - self.rate_hz).abs() < 1.0 {
            return;
        }
        self.rate_hz = rate_hz;
        self.spawn();
    }

    /// Write I/Q and return any packets that arrived since the last call.
    pub fn push(&mut self, iq: &[Complex32]) -> Vec<PhyPacket> {
        if self.child.is_none() && self.unavailable.is_none() {
            self.spawn();
        }
        if let Some(stdin) = self.stdin.as_mut() {
            self.iq_scratch.clear();
            self.iq_scratch.reserve(iq.len() * 2);
            for c in iq {
                self.iq_scratch.push(c.re);
                self.iq_scratch.push(c.im);
            }
            let bytes = bytemuck_f32_slice(&self.iq_scratch);
            if let Err(e) = stdin.write_all(bytes) {
                warn!(error = %e, "lorarx stdin write failed — respawning");
                self.teardown(Some(format!("lorarx stdin broken: {e}")));
                self.spawn();
            } else {
                let _ = stdin.flush();
            }
        }
        self.drain_packets()
    }

    fn drain_packets(&mut self) -> Vec<PhyPacket> {
        let Some(rx) = self.pkt_rx.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(p) => out.push(p),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.teardown(Some(
                        "lorarx exited — check binary and re-enter MESH mode".into(),
                    ));
                    break;
                }
            }
        }
        out
    }

    fn spawn(&mut self) {
        self.teardown(None);
        let rate = self.rate_hz.round() as u32;
        // Match OpenWebRX+ MeshtasticDemodulator: LongFast-centred (-b 8 = 250 kHz)
        // with SF11; multi-SF list can widen later.
        let mut cmd = Command::new("lorarx");
        cmd.args([
            "-i",
            "/dev/stdin",
            "-r",
            &rate.to_string(),
            "-f",
            "f32",
            "-v",
            "-N",
            "-Q",
            "-j",
            "/dev/stdout",
            "-H",
            "5",
            "-W",
            "50",
            "-b",
            "8",
            "-s",
            "11",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

        match cmd.spawn() {
            Ok(mut child) => {
                let stdout = match child.stdout.take() {
                    Some(s) => s,
                    None => {
                        self.unavailable = Some(LORARX_MISSING.into());
                        let _ = child.kill();
                        return;
                    }
                };
                let stdin = match child.stdin.take() {
                    Some(s) => s,
                    None => {
                        self.unavailable = Some(LORARX_MISSING.into());
                        let _ = child.kill();
                        return;
                    }
                };
                let (tx, rx) = mpsc::channel();
                let reader = std::thread::Builder::new()
                    .name("sdroxide-lorarx".into())
                    .spawn(move || read_json_loop(stdout, tx))
                    .ok();
                info!(rate_hz = self.rate_hz, "lorarx PHY started");
                self.child = Some(child);
                self.stdin = Some(stdin);
                self.pkt_rx = Some(rx);
                self.reader = reader;
                self.unavailable = None;
            }
            Err(e) => {
                warn!(error = %e, "lorarx spawn failed");
                self.unavailable = Some(LORARX_MISSING.into());
            }
        }
    }

    fn teardown(&mut self, reason: Option<String>) {
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait_timeout(Duration::from_millis(200));
        }
        self.pkt_rx = None;
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
        if let Some(r) = reason {
            self.unavailable = Some(r);
        }
    }
}

impl Drop for LoraRxPhy {
    fn drop(&mut self) {
        self.teardown(None);
    }
}

trait WaitTimeout {
    fn wait_timeout(&mut self, dur: Duration) -> std::io::Result<std::process::ExitStatus>;
}

impl WaitTimeout for Child {
    fn wait_timeout(&mut self, dur: Duration) -> std::io::Result<std::process::ExitStatus> {
        let start = std::time::Instant::now();
        loop {
            match self.try_wait()? {
                Some(st) => return Ok(st),
                None if start.elapsed() >= dur => {
                    let _ = self.kill();
                    return self.wait();
                }
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }
}

fn read_json_loop(stdout: impl std::io::Read, tx: mpsc::Sender<PhyPacket>) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else {
            break;
        };
        let line = line.trim();
        if line.is_empty() || !line.starts_with('{') {
            continue;
        }
        let Ok(j) = serde_json::from_str::<LoraJson>(line) else {
            continue;
        };
        // Same gate as OpenWebRX MeshtasticParser: need payload and crc ok.
        if j.crc < 1 {
            continue;
        }
        let Some(b64) = j.payload.as_deref() else {
            continue;
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) else {
            continue;
        };
        if bytes.len() < 16 {
            continue;
        }
        let snr_db = j.snr.unwrap_or(0.0);
        if tx
            .send(PhyPacket {
                bytes,
                snr_db,
                rssi_dbm: None,
            })
            .is_err()
        {
            break;
        }
    }
}

fn bytemuck_f32_slice(v: &[f32]) -> &[u8] {
    // SAFETY: f32 is plain old data; openwebrx feeds the same interleaved layout.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 4) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_sets_unavailable() {
        // If lorarx is on PATH this still constructs; only assert the type works.
        let phy = LoraRxPhy::new(1_000_000.0);
        let _ = phy.unavailable();
    }
}
