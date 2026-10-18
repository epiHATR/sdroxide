//! Decoded VDL2 frames as dumpvdl2-compatible JSON over UDP.
//!
//! Fire-and-forget: a listener that is not running must never stall the radio.
//! Native only — binds a UDP socket.

use std::net::{ToSocketAddrs, UdpSocket};

use sdroxide_types::{Vdl2Message, Vdl2UdpConfig};
use tracing::{debug, info};

use crate::json_export;

/// A configured sender.
pub struct Vdl2JsonUdp {
    sock: UdpSocket,
    dest: std::net::SocketAddr,
    addr: String,
}

impl Vdl2JsonUdp {
    /// Bind a socket and resolve the destination.
    pub fn start(cfg: &Vdl2UdpConfig) -> Result<Self, String> {
        let host = if cfg.host.trim().is_empty() { "127.0.0.1" } else { cfg.host.trim() };
        let dest = (host, cfg.port)
            .to_socket_addrs()
            .map_err(|e| format!("{host}:{}: {e}", cfg.port))?
            .next()
            .ok_or_else(|| format!("{host}:{}: no address", cfg.port))?;
        let sock = UdpSocket::bind(if dest.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" })
            .map_err(|e| e.to_string())?;
        sock.set_nonblocking(true).map_err(|e| e.to_string())?;
        if dest.is_ipv4() {
            let _ = sock.set_broadcast(true);
        }
        if dest.is_ipv4() && dest.ip().is_multicast() {
            let _ = sock.set_multicast_ttl_v4(2);
        }
        let addr = format!("{host}:{}", cfg.port);
        info!(dest = %addr, "VDL2 JSON UDP output started");
        Ok(Vdl2JsonUdp { sock, dest, addr })
    }

    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// Send one frame. Empty payloads are not produced by the formatter.
    pub fn send(&self, msg: &Vdl2Message, station: Option<&str>) {
        let line = json_export::message_json(msg, station);
        if let Err(e) = self.sock.send_to(line.as_bytes(), self.dest) {
            debug!(dest = %self.addr, error = %e, "VDL2 JSON UDP send failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdroxide_types::{Vdl2AddrKind, Vdl2Frame, Vdl2Message, Vdl2Payload};
    use std::time::Duration;

    #[test]
    fn json_datagram_arrives() {
        let listener = UdpSocket::bind("127.0.0.1:0").unwrap();
        listener.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let port = listener.local_addr().unwrap().port();
        let cfg = Vdl2UdpConfig { enabled: true, host: "127.0.0.1".into(), port };
        let tx = Vdl2JsonUdp::start(&cfg).unwrap();
        let msg = Vdl2Message {
            at: 100,
            freq_hz: 136_975_000.0,
            src: 1,
            src_kind: Vdl2AddrKind::Aircraft,
            dst: 2,
            dst_kind: Vdl2AddrKind::GroundAdmin,
            command: true,
            frame: Vdl2Frame::Ui { pf: false },
            payload: Vdl2Payload::None,
            snr_db: 10.0,
            rssi_dbfs: -40.0,
            evm_deg: 1.0,
            freq_err_hz: 0.0,
            rs_corrected: 0,
            raw_hex: "AABB".into(),
        };
        tx.send(&msg, None);
        let mut buf = [0u8; 4096];
        let (n, _) = listener.recv_from(&mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf[..n]).unwrap();
        assert_eq!(v["vdl2"]["freq"], 136_975_000);
        assert_eq!(v["vdl2"]["avlc"]["frame_type"], "UI");
    }
}
