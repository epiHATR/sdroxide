//! Minimal protobuf wire reader for the handful of Meshtastic fields we need.
//!
//! Prefer this over pulling the full `meshtastic` prost surface into the native
//! demod crate — we only touch `Data` / `User` / `Position` / `DeviceMetrics`.

/// Port numbers we care about (firmware `portnums.proto`).
pub const PORT_TEXT_MESSAGE: u32 = 1;
pub const PORT_POSITION: u32 = 3;
pub const PORT_NODEINFO: u32 = 4;
pub const PORT_TELEMETRY: u32 = 67;

/// Decoded `mesh::Data` envelope after AES unwrap.
#[derive(Debug, Clone, Default)]
pub struct MeshData {
    pub portnum: u32,
    pub payload: Vec<u8>,
}

/// `mesh::User` (NODEINFO_APP).
#[derive(Debug, Clone, Default)]
pub struct User {
    pub id: String,
    pub long_name: String,
    pub short_name: String,
}

/// `mesh::Position` (POSITION_APP) — degrees when both ints present.
#[derive(Debug, Clone, Default)]
pub struct Position {
    pub lat: Option<f64>,
    pub lon: Option<f64>,
}

/// Battery percent from `telemetry::DeviceMetrics` inside TELEMETRY_APP.
#[derive(Debug, Clone, Default)]
pub struct DeviceMetrics {
    pub battery_pct: Option<u8>,
}

/// Parse a `Data` message from decrypted payload bytes.
pub fn parse_data(bytes: &[u8]) -> Option<MeshData> {
    let mut out = MeshData::default();
    let mut i = 0;
    while i < bytes.len() {
        let (tag, ni) = read_varint(bytes, i)?;
        i = ni;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u8;
        match (field, wire) {
            (1, 0) => {
                let (v, ni) = read_varint(bytes, i)?;
                out.portnum = v as u32;
                i = ni;
            }
            (2, 2) => {
                let (len, ni) = read_varint(bytes, i)?;
                i = ni;
                let end = i + len as usize;
                if end > bytes.len() {
                    return None;
                }
                out.payload = bytes[i..end].to_vec();
                i = end;
            }
            _ => {
                i = skip_field(bytes, i, wire)?;
            }
        }
    }
    Some(out)
}

pub fn parse_user(bytes: &[u8]) -> User {
    let mut out = User::default();
    let mut i = 0;
    while i < bytes.len() {
        let Some((tag, ni)) = read_varint(bytes, i) else {
            break;
        };
        i = ni;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u8;
        match (field, wire) {
            (1, 2) => {
                if let Some((s, ni)) = read_string(bytes, i) {
                    out.id = s;
                    i = ni;
                } else {
                    break;
                }
            }
            (2, 2) => {
                if let Some((s, ni)) = read_string(bytes, i) {
                    out.long_name = s;
                    i = ni;
                } else {
                    break;
                }
            }
            (3, 2) => {
                if let Some((s, ni)) = read_string(bytes, i) {
                    out.short_name = s;
                    i = ni;
                } else {
                    break;
                }
            }
            _ => {
                if let Some(ni) = skip_field(bytes, i, wire) {
                    i = ni;
                } else {
                    break;
                }
            }
        }
    }
    out
}

pub fn parse_position(bytes: &[u8]) -> Position {
    let mut lat_i: Option<i32> = None;
    let mut lon_i: Option<i32> = None;
    let mut i = 0;
    while i < bytes.len() {
        let Some((tag, ni)) = read_varint(bytes, i) else {
            break;
        };
        i = ni;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u8;
        match (field, wire) {
            // sfixed32
            (1, 5) => {
                if i + 4 > bytes.len() {
                    break;
                }
                lat_i = Some(i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()));
                i += 4;
            }
            (2, 5) => {
                if i + 4 > bytes.len() {
                    break;
                }
                lon_i = Some(i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()));
                i += 4;
            }
            _ => {
                if let Some(ni) = skip_field(bytes, i, wire) {
                    i = ni;
                } else {
                    break;
                }
            }
        }
    }
    Position {
        lat: lat_i.map(|v| f64::from(v) * 1e-7),
        lon: lon_i.map(|v| f64::from(v) * 1e-7),
    }
}

pub fn parse_telemetry_battery(bytes: &[u8]) -> DeviceMetrics {
    // Telemetry { device_metrics = 2 { battery_level = 1 } }
    let mut out = DeviceMetrics::default();
    let mut i = 0;
    while i < bytes.len() {
        let Some((tag, ni)) = read_varint(bytes, i) else {
            break;
        };
        i = ni;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u8;
        if field == 2 && wire == 2 {
            let Some((len, ni)) = read_varint(bytes, i) else {
                break;
            };
            i = ni;
            let end = i + len as usize;
            if end > bytes.len() {
                break;
            }
            out.battery_pct = parse_device_metrics_battery(&bytes[i..end]);
            i = end;
        } else if let Some(ni) = skip_field(bytes, i, wire) {
            i = ni;
        } else {
            break;
        }
    }
    out
}

fn parse_device_metrics_battery(bytes: &[u8]) -> Option<u8> {
    let mut i = 0;
    while i < bytes.len() {
        let (tag, ni) = read_varint(bytes, i)?;
        i = ni;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u8;
        if field == 1 && wire == 0 {
            let (v, _) = read_varint(bytes, i)?;
            // 0..=100 normal; 101 means powered / unknown in some firmware.
            if v <= 100 {
                return Some(v as u8);
            }
            return None;
        }
        i = skip_field(bytes, i, wire)?;
    }
    None
}

fn read_varint(bytes: &[u8], mut i: usize) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        if i >= bytes.len() || shift >= 64 {
            return None;
        }
        let b = bytes[i];
        i += 1;
        result |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some((result, i));
        }
        shift += 7;
    }
}

fn read_string(bytes: &[u8], i: usize) -> Option<(String, usize)> {
    let (len, ni) = read_varint(bytes, i)?;
    let end = ni + len as usize;
    if end > bytes.len() {
        return None;
    }
    let s = String::from_utf8_lossy(&bytes[ni..end]).into_owned();
    Some((s, end))
}

fn skip_field(bytes: &[u8], i: usize, wire: u8) -> Option<usize> {
    match wire {
        0 => read_varint(bytes, i).map(|(_, ni)| ni),
        1 => (i + 8 <= bytes.len()).then_some(i + 8),
        2 => {
            let (len, ni) = read_varint(bytes, i)?;
            let end = ni + len as usize;
            (end <= bytes.len()).then_some(end)
        }
        5 => (i + 4 <= bytes.len()).then_some(i + 4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_data() {
        // field 1 varint = 1 (TEXT), field 2 len=5 "hello"
        let bytes = [0x08, 0x01, 0x12, 0x05, b'h', b'e', b'l', b'l', b'o'];
        let d = parse_data(&bytes).unwrap();
        assert_eq!(d.portnum, PORT_TEXT_MESSAGE);
        assert_eq!(d.payload, b"hello");
    }

    #[test]
    fn parse_user_names() {
        // id="!abcd", long="Test Node", short="TN"
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x0a, 0x05]);
        bytes.extend_from_slice(b"!abcd");
        bytes.extend_from_slice(&[0x12, 0x09]);
        bytes.extend_from_slice(b"Test Node");
        bytes.extend_from_slice(&[0x1a, 0x02]);
        bytes.extend_from_slice(b"TN");
        let u = parse_user(&bytes);
        assert_eq!(u.id, "!abcd");
        assert_eq!(u.long_name, "Test Node");
        assert_eq!(u.short_name, "TN");
    }

    #[test]
    fn parse_position_degrees() {
        let mut bytes = Vec::new();
        // lat_i = 515000000 → 51.5°, lon_i = -10000000 → -1.0°
        bytes.push(0x0d);
        bytes.extend_from_slice(&515_000_000i32.to_le_bytes());
        bytes.push(0x15);
        bytes.extend_from_slice(&(-10_000_000i32).to_le_bytes());
        let p = parse_position(&bytes);
        assert!((p.lat.unwrap() - 51.5).abs() < 1e-6);
        assert!((p.lon.unwrap() - (-1.0)).abs() < 1e-6);
    }
}
