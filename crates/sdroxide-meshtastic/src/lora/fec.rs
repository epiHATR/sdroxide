//! LoRa Hamming / diagonal interleave / whitening / header — Semtech PHY as
//! documented by LoRaPHY / ACM “Complete LoRa PHY” (jkadbear).
//!
//! Decode order: bin calibrate + Gray → diagonal deinterleave → Hamming →
//! (payload only) dewhiten.

/// Coding rate index: `1` = 4/5 … `4` = 4/8.
pub type CodingRate = u8;

/// Semtech whitening (LFSR x⁸+x⁶+x⁵+x⁴+1, init 0xff).
pub fn whiten(bytes: &mut [u8]) {
    let mut r: u8 = 0xff;
    for b in bytes.iter_mut() {
        *b ^= r;
        let bit = ((r >> 7) ^ (r >> 5) ^ (r >> 4) ^ (r >> 3)) & 1;
        r = (r << 1) | bit;
    }
}

fn gray_decode(v: u32) -> u32 {
    v ^ (v >> 1)
}

/// Header / LDRO symbol from raw FFT bin (LoRaPHY decode `gray_coding`).
pub fn header_symbol_from_bin(bin: u32) -> u32 {
    gray_decode(bin >> 2)
}

/// Payload symbol from raw FFT bin when LDRO is off.
pub fn payload_symbol_from_bin(bin: u32, sf: u8) -> u32 {
    let mask = (1u32 << sf) - 1;
    gray_decode(bin.wrapping_sub(1) & mask)
}

/// Diagonal deinterleave (LoRaPHY `diag_deinterleave`).
///
/// `ppm` = SF or SF−2. Returns `ppm` codewords (one per bit column).
pub fn diag_deinterleave(symbols: &[u32], ppm: usize) -> Vec<u8> {
    let n = symbols.len();
    if n == 0 || ppm == 0 {
        return Vec::new();
    }
    // Each symbol → ppm bits, left-msb into a row; left-rotate row by row index.
    let mut shifted = vec![vec![0u8; ppm]; n];
    for (i, &sym) in symbols.iter().enumerate() {
        for bit in 0..ppm {
            let src = ppm - 1 - bit; // left-msb
            shifted[i][bit] = ((sym >> src) & 1) as u8;
        }
        let rot = i % ppm;
        if rot > 0 {
            shifted[i].rotate_left(rot);
        }
    }
    // Columns → codewords (MATLAB bi2de: first row = LSB), then flipud.
    let mut cols = Vec::with_capacity(ppm);
    for col in 0..ppm {
        let mut cw = 0u8;
        for row in 0..n {
            cw |= shifted[row][col] << row;
        }
        cols.push(cw);
    }
    cols.reverse();
    cols
}

/// Hamming decode → data nibbles. `rdd` = 4+CR ∈ {5,6,7,8}.
pub fn hamming_decode(codewords: &[u8], rdd: u8) -> Vec<u8> {
    let mut nibbles = Vec::with_capacity(codewords.len());
    for &cw in codewords {
        let mut c = u16::from(cw);
        if rdd >= 7 {
            let p2 = parity_bits(c, &[7, 4, 2, 1]);
            let p3 = parity_bits(c, &[5, 3, 2, 1]);
            let p5 = parity_bits(c, &[6, 4, 3, 2]);
            let parity = p2 * 4 + p3 * 2 + p5;
            let pf = match parity {
                3 => 4,
                5 => 8,
                6 => 1,
                7 => 2,
                _ => 0,
            };
            c ^= pf;
        }
        nibbles.push((c & 0x0f) as u8);
    }
    nibbles
}

fn parity_bits(cw: u16, pos: &[u8]) -> u16 {
    let mut x = 0u16;
    for &p in pos {
        if cw & (1 << (p - 1)) != 0 {
            x ^= 1;
        }
    }
    x
}

/// Pack nibbles into bytes with the first nibble as the high nibble.
pub fn nibbles_to_bytes(nibbles: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(nibbles.len().div_ceil(2));
    let mut i = 0;
    while i + 1 < nibbles.len() {
        bytes.push((nibbles[i] << 4) | (nibbles[i + 1] & 0x0f));
        i += 2;
    }
    if i < nibbles.len() {
        bytes.push(nibbles[i] << 4);
    }
    bytes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExplicitHeader {
    pub payload_len: usize,
    pub cr: CodingRate,
    pub has_crc: bool,
}

/// 5×12 GF(2) header checksum (LoRaPHY).
const HDR_CS: [[u8; 12]; 5] = [
    [1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [1, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 1],
    [0, 1, 0, 0, 1, 0, 0, 1, 1, 0, 1, 0],
    [0, 0, 1, 0, 0, 1, 0, 1, 0, 1, 1, 1],
    [0, 0, 0, 1, 0, 0, 1, 0, 1, 1, 1, 1],
];

pub fn header_checksum_ok(nibbles: &[u8]) -> bool {
    if nibbles.len() < 5 {
        return false;
    }
    let mut bits = [0u8; 12];
    for n in 0..3 {
        for b in 0..4 {
            bits[n * 4 + b] = (nibbles[n] >> (3 - b)) & 1;
        }
    }
    let mut calc = [0u8; 5];
    for r in 0..5 {
        let mut s = 0u8;
        for c in 0..12 {
            s ^= HDR_CS[r][c] & bits[c];
        }
        calc[r] = s;
    }
    let mut got = [0u8; 5];
    got[0] = nibbles[3] & 1;
    for b in 0..4 {
        got[b + 1] = (nibbles[4] >> (3 - b)) & 1;
    }
    calc == got
}

/// Decode 8 Gray-mapped header symbols (SF−2 bits each).
///
/// Returns the header fields plus any payload nibbles that rode in the same
/// 8-symbol block (SF−2 codewords − 5 header nibbles). Those must seed the
/// payload decode — the Semtech symbol-count formula already accounts for them.
pub fn decode_explicit_header(
    symbols_g: &[u32],
    sf: u8,
) -> Option<(ExplicitHeader, Vec<u8>)> {
    if symbols_g.len() < 8 || sf < 2 {
        return None;
    }
    let ppm = sf as usize - 2;
    let codewords = diag_deinterleave(&symbols_g[..8], ppm);
    // Header always uses CR 4/8 → rdd=8. Codeword count = ppm (=SF-2).
    let nibbles = hamming_decode(&codewords, 8);
    if nibbles.len() < 5 || !header_checksum_ok(&nibbles) {
        return None;
    }
    let payload_len = ((nibbles[0] as usize) << 4) | (nibbles[1] as usize);
    let cr = (nibbles[2] >> 1).clamp(1, 4);
    let has_crc = nibbles[2] & 1 != 0;
    // Meshtastic LongFast: CR 4/5, CRC on, mesh header ≥16 bytes. Reject
    // checksum collisions that claim other modem configs.
    if payload_len < 16 || payload_len > 128 || cr != 1 || !has_crc {
        return None;
    }
    let leftover = nibbles[5..].to_vec();
    Some((
        ExplicitHeader {
            payload_len,
            cr,
            has_crc,
        },
        leftover,
    ))
}

/// Payload symbols after the 8-symbol explicit header (LoRaPHY / Semtech).
pub fn payload_symbols_after_header(
    payload_len: usize,
    has_crc: bool,
    sf: u8,
    cr: CodingRate,
    ldr: bool,
) -> usize {
    let de = if ldr { 1i32 } else { 0 };
    let crc = if has_crc { 1i32 } else { 0 };
    let num = 2 * payload_len as i32 - sf as i32 + 7 + 4 * crc;
    let den = (sf as i32 - 2 * de).max(1);
    let blocks = ((num + den - 1) / den).max(0);
    (blocks * (cr as i32 + 4)).max(0) as usize
}

pub fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitening_matches_loraphy_prefix() {
        let mut z = [0u8; 6];
        whiten(&mut z);
        assert_eq!(&z, &[0xff, 0xfe, 0xfc, 0xf8, 0xf0, 0xe1]);
    }

    #[test]
    fn header_checksum_synthetic() {
        let mut nibbles = [0u8; 5];
        let plen = 20u8;
        nibbles[0] = plen >> 4;
        nibbles[1] = plen & 0x0f;
        nibbles[2] = (1 << 1) | 1;
        let mut bits = [0u8; 12];
        for n in 0..3 {
            for b in 0..4 {
                bits[n * 4 + b] = (nibbles[n] >> (3 - b)) & 1;
            }
        }
        let mut calc = [0u8; 5];
        for r in 0..5 {
            let mut s = 0u8;
            for c in 0..12 {
                s ^= HDR_CS[r][c] & bits[c];
            }
            calc[r] = s;
        }
        nibbles[3] = calc[0];
        nibbles[4] = (calc[1] << 3) | (calc[2] << 2) | (calc[3] << 1) | calc[4];
        assert!(header_checksum_ok(&nibbles));
    }

    #[test]
    fn payload_symbol_count_longfast() {
        let n = payload_symbols_after_header(16, true, 11, 1, false);
        assert!(n > 0);
        assert_eq!(n % 5, 0);
    }
}
