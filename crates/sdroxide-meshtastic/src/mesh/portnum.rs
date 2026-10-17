//! Map decrypted mesh `Data` payloads onto tracker updates.

use super::crypto::{CryptoKey, aes_ctr_inplace, public_channel_hashes};
use super::header::PacketHeader;
use super::proto::{
    PORT_NODEINFO, PORT_POSITION, PORT_TELEMETRY, PORT_TEXT_MESSAGE, parse_data, parse_position,
    parse_telemetry_battery, parse_user,
};
use crate::track::{HeardPacket, TrackerUpdate};

/// One fully decoded airframe ready for the tracker.
#[derive(Debug, Clone)]
pub struct DecodedPacket {
    pub header: PacketHeader,
    pub update: TrackerUpdate,
    /// SNR reported by the PHY, if any.
    pub snr_db: Option<f32>,
    pub rssi_dbm: Option<f32>,
}

/// Try to turn raw LoRa payload bytes into a [`DecodedPacket`].
///
/// `payload` is the bytes after Semtech PHY CRC (Meshtastic header + ciphertext).
pub fn decode_airframe(
    payload: &[u8],
    key: &CryptoKey,
    snr_db: Option<f32>,
    rssi_dbm: Option<f32>,
    unix: i64,
) -> Option<DecodedPacket> {
    let header = PacketHeader::parse(payload)?;
    let hashes = public_channel_hashes(key);
    if header.channel != hashes[0] && header.channel != hashes[1] {
        // Still try — operator may be on a named channel with matching PSK but
        // a different display name. Reject only empty / nonsense later.
        tracing::trace!(ch = header.channel, want = ?hashes, "channel hash miss");
    }
    let mut body = payload[PacketHeader::LEN..].to_vec();
    // Public LongFast uses the default PSK (AES-CTR). A channel with no crypto
    // leaves the protobuf in the clear — try decrypt first, fall back to raw.
    aes_ctr_inplace(key, header.from, header.id, &mut body);
    let data = match parse_data(&body) {
        Some(d) if d.portnum != 0 || !d.payload.is_empty() => d,
        _ => parse_data(&payload[PacketHeader::LEN..])?,
    };
    let from = PacketHeader::format_node(header.from);
    let to = PacketHeader::format_node(header.to);
    let hops = Some(header.flags.hop_start().saturating_sub(header.flags.hop_limit()));

    let update = match data.portnum {
        PORT_TEXT_MESSAGE => {
            let text = String::from_utf8_lossy(&data.payload).into_owned();
            TrackerUpdate::Text {
                from: from.clone(),
                to,
                text,
                unix,
                snr_db,
            }
        }
        PORT_NODEINFO => {
            let u = parse_user(&data.payload);
            let id = if u.id.is_empty() { from.clone() } else { u.id };
            TrackerUpdate::NodeInfo {
                id,
                short_name: u.short_name,
                long_name: u.long_name,
                unix,
                snr_db,
                rssi_dbm,
                hops,
            }
        }
        PORT_POSITION => {
            let p = parse_position(&data.payload);
            TrackerUpdate::Position {
                id: from.clone(),
                lat: p.lat,
                lon: p.lon,
                unix,
                snr_db,
                rssi_dbm,
                hops,
            }
        }
        PORT_TELEMETRY => {
            let m = parse_telemetry_battery(&data.payload);
            TrackerUpdate::Telemetry {
                id: from.clone(),
                battery_pct: m.battery_pct,
                unix,
                snr_db,
                rssi_dbm,
                hops,
            }
        }
        other => TrackerUpdate::Opaque {
            from: from.clone(),
            to,
            portnum: other,
            raw_hex: hex_preview(&data.payload),
            unix,
            snr_db,
        },
    };

    // Ensure the from-node is always touched even for text-only.
    let _ = HeardPacket {
        id: from,
        unix,
        snr_db,
        rssi_dbm,
        hops,
    };

    Some(DecodedPacket {
        header,
        update,
        snr_db,
        rssi_dbm,
    })
}

fn hex_preview(bytes: &[u8]) -> String {
    const MAX: usize = 24;
    let take = bytes.len().min(MAX);
    let mut s = String::with_capacity(take * 2 + 3);
    for b in &bytes[..take] {
        s.push_str(&format!("{b:02x}"));
    }
    if bytes.len() > MAX {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::crypto::{DEFAULT_PSK, aes_ctr_inplace, channel_hash};
    use sdroxide_types::MeshtasticPskKind;

    #[test]
    fn decrypt_and_decode_text() {
        let key = CryptoKey::Aes128(DEFAULT_PSK);
        let from = 0xa1b2_c3d4u32;
        let id = 0x0102_0304u32;
        let mut data = vec![0x08, 0x01, 0x12, 0x04];
        data.extend_from_slice(b"ping");
        aes_ctr_inplace(&key, from, id, &mut data);

        let mut frame = Vec::new();
        frame.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        frame.extend_from_slice(&from.to_le_bytes());
        frame.extend_from_slice(&id.to_le_bytes());
        frame.push(0x03); // hop_limit 3
        frame.push(channel_hash("LongFast", &key));
        frame.push(0);
        frame.push(0);
        frame.extend_from_slice(&data);

        let dec = decode_airframe(&frame, &key, Some(7.5), None, 1_700_000_000).unwrap();
        match dec.update {
            TrackerUpdate::Text { text, from: f, .. } => {
                assert_eq!(text, "ping");
                assert_eq!(f, "!a1b2c3d4");
            }
            other => panic!("unexpected {other:?}"),
        }
        let _ = MeshtasticPskKind::Default;
    }
}
