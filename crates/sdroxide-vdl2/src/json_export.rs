//! dumpvdl2-compatible JSON for one decoded AVLC frame.
//!
//! Shape matches what dumpvdl2 emits for `decoded:json:…` (root key `vdl2`,
//! nested `app` / `t` / `freq` / RF stats / `avlc`). Built from our
//! [`Vdl2Message`] — AVLC + ACARS + the XID subset we decode; no libacars
//! ATN trees. `app.name` is `sdroxide`, not a spoof of dumpvdl2.

use serde_json::{Value, json};
use sdroxide_types::{Vdl2AddrKind, Vdl2Frame, Vdl2Message, Vdl2Payload};

/// One UDP / file line: compact JSON for `msg`, optionally tagged with the
/// station callsign dumpvdl2 would put in `station`.
pub fn message_json(msg: &Vdl2Message, station: Option<&str>) -> String {
    let mut vdl2 = json!({
        "app": {
            "name": "sdroxide",
            "ver": env!("CARGO_PKG_VERSION"),
        },
        "t": {
            "sec": msg.at,
            "usec": 0,
        },
        "freq": msg.freq_hz.round() as i64,
        "burst_len_octets": burst_len_octets(&msg.raw_hex),
        "hdr_bits_fixed": 0,
        "octets_corrected_by_fec": msg.rs_corrected,
        "idx": 0,
        "sig_level": msg.rssi_dbfs as f64,
        "noise_level": (msg.rssi_dbfs - msg.snr_db) as f64,
        // Hz residual on this burst (dumpvdl2 often reports ppm; consumers
        // that only display the number still get a useful offset).
        "freq_skew": msg.freq_err_hz as f64,
        "avlc": avlc_object(msg),
    });
    if let Some(s) = station.map(str::trim).filter(|s| !s.is_empty()) {
        vdl2["station"] = Value::String(s.to_string());
    }
    json!({ "vdl2": vdl2 }).to_string()
}

fn burst_len_octets(raw_hex: &str) -> usize {
    raw_hex.chars().filter(|c| c.is_ascii_hexdigit()).count() / 2
}

fn avlc_object(msg: &Vdl2Message) -> Value {
    let mut avlc = json!({
        "src": addr_object(msg.src, msg.src_kind),
        "dst": addr_object(msg.dst, msg.dst_kind),
        "cr": if msg.command { "Command" } else { "Response" },
        "frame_type": frame_type_label(msg.frame),
    });
    match msg.frame {
        Vdl2Frame::I { ns, nr, p } => {
            avlc["rseq"] = json!(nr);
            avlc["sseq"] = json!(ns);
            avlc["poll"] = json!(p);
        }
        Vdl2Frame::Rr { nr, pf }
        | Vdl2Frame::Rnr { nr, pf }
        | Vdl2Frame::Rej { nr, pf }
        | Vdl2Frame::Srej { nr, pf } => {
            avlc["rseq"] = json!(nr);
            avlc["poll"] = json!(pf);
        }
        Vdl2Frame::Ui { pf }
        | Vdl2Frame::Dm { pf }
        | Vdl2Frame::Disc { pf }
        | Vdl2Frame::Ua { pf }
        | Vdl2Frame::Frmr { pf }
        | Vdl2Frame::Xid { pf }
        | Vdl2Frame::Test { pf } => {
            avlc["poll"] = json!(pf);
        }
        Vdl2Frame::Unknown(_) => {}
    }
    match &msg.payload {
        Vdl2Payload::Acars(a) => {
            avlc["acars"] = acars_object(a);
        }
        Vdl2Payload::Xid(x) => {
            avlc["xid"] = xid_object(x);
        }
        Vdl2Payload::Other { note, hex } => {
            avlc["other"] = json!({ "note": note, "hex": hex });
        }
        Vdl2Payload::None => {}
    }
    avlc
}

fn addr_object(addr: u32, kind: Vdl2AddrKind) -> Value {
    json!({
        "addr": format!("{:06X}", addr & 0xff_ffff),
        "type": dumpvdl2_addr_type(kind),
    })
}

fn dumpvdl2_addr_type(kind: Vdl2AddrKind) -> &'static str {
    match kind {
        Vdl2AddrKind::Aircraft => "Aircraft",
        Vdl2AddrKind::GroundAdmin | Vdl2AddrKind::GroundDelegated => "Ground station",
        Vdl2AddrKind::AllStations => "All stations",
        Vdl2AddrKind::Reserved => "Reserved",
    }
}

fn frame_type_label(frame: Vdl2Frame) -> &'static str {
    match frame {
        Vdl2Frame::I { .. } => "I",
        Vdl2Frame::Rr { .. } => "RR",
        Vdl2Frame::Rnr { .. } => "RNR",
        Vdl2Frame::Rej { .. } => "REJ",
        Vdl2Frame::Srej { .. } => "SREJ",
        Vdl2Frame::Ui { .. } => "UI",
        Vdl2Frame::Dm { .. } => "DM",
        Vdl2Frame::Disc { .. } => "DISC",
        Vdl2Frame::Ua { .. } => "UA",
        Vdl2Frame::Frmr { .. } => "FRMR",
        Vdl2Frame::Xid { .. } => "XID",
        Vdl2Frame::Test { .. } => "TEST",
        Vdl2Frame::Unknown(_) => "?",
    }
}

fn acars_object(a: &sdroxide_types::Vdl2Acars) -> Value {
    let reg = {
        let t = a.registration.trim();
        if t.is_empty() {
            String::new()
        } else if t.starts_with('.') {
            t.to_string()
        } else {
            format!(".{t}")
        }
    };
    let msn = a.msn.trim();
    let (msg_num, msg_num_seq) = if msn.len() >= 4 {
        (msn[..msn.len() - 1].to_string(), msn[msn.len() - 1..].to_string())
    } else if msn.is_empty() {
        (String::new(), String::new())
    } else {
        (msn.to_string(), String::new())
    };
    json!({
        "err": a.parity_errors > 0,
        "crc_ok": a.crc_ok,
        "more": a.more,
        "reg": reg,
        "mode": a.mode.to_string(),
        "label": a.label,
        "blk_id": a.block_id.to_string(),
        "ack": a.ack.to_string(),
        "flight": a.flight.trim(),
        "msg_num": msg_num,
        "msg_num_seq": msg_num_seq,
        "msg_text": a.text,
    })
}

fn xid_object(x: &sdroxide_types::Vdl2Xid) -> Value {
    let mut o = json!({
        "kind": x.kind,
        "params": x.params.iter().map(|(k, v)| json!({"name": k, "value": v})).collect::<Vec<_>>(),
        "unknown": x.unknown,
    });
    if let (Some(lat), Some(lon)) = (x.lat, x.lon) {
        o["lat"] = json!(lat);
        o["lon"] = json!(lon);
    }
    if !x.frequencies.is_empty() {
        o["frequencies"] = json!(x.frequencies);
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdroxide_types::{Vdl2Acars, Vdl2AddrKind, Vdl2Frame, Vdl2Message, Vdl2Payload};

    fn sample_acars() -> Vdl2Message {
        Vdl2Message {
            at: 1_632_024_554,
            freq_hz: 136_975_000.0,
            src: 0x00A7_DB08,
            src_kind: Vdl2AddrKind::Aircraft,
            dst: 0x0010_502A,
            dst_kind: Vdl2AddrKind::GroundAdmin,
            command: true,
            frame: Vdl2Frame::I { ns: 0, nr: 1, p: false },
            payload: Vdl2Payload::Acars(Vdl2Acars {
                mode: '2',
                registration: "N605NK".into(),
                ack: '!',
                label: "12".into(),
                block_id: '5',
                msn: "M80A".into(),
                flight: "NK0626".into(),
                text: "POSN test".into(),
                more: false,
                crc_ok: true,
                parity_errors: 0,
            }),
            snr_db: 18.0,
            rssi_dbfs: -27.2,
            evm_deg: 5.0,
            freq_err_hz: -3.4,
            rs_corrected: 0,
            raw_hex: "00".repeat(126),
        }
    }

    #[test]
    fn dumpvdl2_shaped_acars_json() {
        let s = message_json(&sample_acars(), Some("N0CALL"));
        let v: Value = serde_json::from_str(&s).unwrap();
        let root = &v["vdl2"];
        assert_eq!(root["app"]["name"], "sdroxide");
        assert_eq!(root["station"], "N0CALL");
        assert_eq!(root["t"]["sec"], 1_632_024_554);
        assert_eq!(root["freq"], 136_975_000);
        assert_eq!(root["burst_len_octets"], 126);
        assert_eq!(root["avlc"]["src"]["addr"], "A7DB08");
        assert_eq!(root["avlc"]["src"]["type"], "Aircraft");
        assert_eq!(root["avlc"]["dst"]["type"], "Ground station");
        assert_eq!(root["avlc"]["cr"], "Command");
        assert_eq!(root["avlc"]["frame_type"], "I");
        assert_eq!(root["avlc"]["acars"]["reg"], ".N605NK");
        assert_eq!(root["avlc"]["acars"]["flight"], "NK0626");
        assert_eq!(root["avlc"]["acars"]["msg_num"], "M80");
        assert_eq!(root["avlc"]["acars"]["msg_num_seq"], "A");
        assert_eq!(root["avlc"]["acars"]["msg_text"], "POSN test");
        assert_eq!(root["avlc"]["acars"]["crc_ok"], true);
    }
}
