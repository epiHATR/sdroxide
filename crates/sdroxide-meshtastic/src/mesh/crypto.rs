//! Meshtastic channel PSK expansion and AES-CTR packet crypto.
//!
//! Constants and nonce layout from firmware `Channels.h` / `CryptoEngine.cpp`.

use aes::Aes128;
use aes::Aes256;
use cipher::{KeyIvInit, StreamCipher};
use ctr::Ctr128BE;
use sdroxide_types::{MeshtasticPskKind, MeshtasticSettings};

type Aes128Ctr = Ctr128BE<Aes128>;
type Aes256Ctr = Ctr128BE<Aes256>;

/// Public default LongFast / MediumFast AES-128 key (firmware `defaultpsk`).
pub const DEFAULT_PSK: [u8; 16] = [
    0xd4, 0xf1, 0xbb, 0x3a, 0x20, 0x29, 0x07, 0x59, 0xf0, 0xbc, 0xff, 0xab, 0xcf, 0x4e, 0x69, 0x01,
];

/// Expanded key ready for AES-CTR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptoKey {
    None,
    Aes128([u8; 16]),
    Aes256([u8; 32]),
}

impl CryptoKey {
    pub fn from_settings(cfg: &MeshtasticSettings) -> Self {
        match cfg.psk_kind {
            MeshtasticPskKind::Default => CryptoKey::Aes128(DEFAULT_PSK),
            MeshtasticPskKind::Simple => {
                let mut k = DEFAULT_PSK;
                let n = cfg.psk_simple.clamp(1, 10);
                // firmware single-byte PSK: copy default, then
                // `bytes[15] = (default[15] + pskIndex) % 256` with the 1-byte
                // form storing `pskIndex`. UI `simpleN` maps to index `N`:
                // simple1 → index 1 → last byte 0x02… wait: Meshtastic docs
                // treat simple1 as the *default* key unchanged. Firmware does:
                //   memcpy(default); bytes[15] += (pskIndex - 1);
                // so index 1 leaves 0x01, index 2 → 0x02, …
                if n >= 2 {
                    k[15] = DEFAULT_PSK[15].wrapping_add(n - 1);
                }
                CryptoKey::Aes128(k)
            }
            MeshtasticPskKind::Key16 => {
                let mut k = [0u8; 16];
                k.copy_from_slice(&cfg.psk_bytes[..16]);
                CryptoKey::Aes128(k)
            }
            MeshtasticPskKind::Key32 => CryptoKey::Aes256(cfg.psk_bytes),
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        match self {
            CryptoKey::None => &[],
            CryptoKey::Aes128(k) => k,
            CryptoKey::Aes256(k) => k,
        }
    }
}

/// Channel hash hint (firmware `Channels::generateHash`): XOR of name and key bytes.
pub fn channel_hash(name: &str, key: &CryptoKey) -> u8 {
    let mut h = 0u8;
    for b in name.bytes() {
        h ^= b;
    }
    for b in key.as_slice() {
        h ^= b;
    }
    h
}

/// Acceptable channel hashes for the public LongFast / Default names with `key`.
pub fn public_channel_hashes(key: &CryptoKey) -> [u8; 2] {
    [channel_hash("LongFast", key), channel_hash("Default", key)]
}

/// 16-byte CTR nonce: little-endian `packet_id` as u64 + `from` as u32 + zeros.
pub fn init_nonce(from: u32, packet_id: u32) -> [u8; 16] {
    let mut nonce = [0u8; 16];
    let id64 = u64::from(packet_id);
    nonce[0..8].copy_from_slice(&id64.to_le_bytes());
    nonce[8..12].copy_from_slice(&from.to_le_bytes());
    nonce
}

/// AES-CTR in place (encrypt == decrypt). No-op for [`CryptoKey::None`].
pub fn aes_ctr_inplace(key: &CryptoKey, from: u32, packet_id: u32, bytes: &mut [u8]) {
    if bytes.is_empty() {
        return;
    }
    let nonce = init_nonce(from, packet_id);
    match key {
        CryptoKey::None => {}
        CryptoKey::Aes128(k) => {
            let mut ctr = Aes128Ctr::new(k.into(), &nonce.into());
            ctr.apply_keystream(bytes);
        }
        CryptoKey::Aes256(k) => {
            let mut ctr = Aes256Ctr::new(k.into(), &nonce.into());
            ctr.apply_keystream(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdroxide_types::MeshtasticSettings;

    #[test]
    fn default_channel_hash_is_eight() {
        // Documented: LongFast + default PSK → channel hash 8.
        let key = CryptoKey::Aes128(DEFAULT_PSK);
        assert_eq!(channel_hash("LongFast", &key), 8);
    }

    #[test]
    fn default_psk_from_settings() {
        let k = CryptoKey::from_settings(&MeshtasticSettings::default());
        assert_eq!(k, CryptoKey::Aes128(DEFAULT_PSK));
    }

    #[test]
    fn simple2_changes_last_byte() {
        let mut cfg = MeshtasticSettings::default();
        cfg.psk_kind = MeshtasticPskKind::Simple;
        cfg.psk_simple = 2;
        let CryptoKey::Aes128(k) = CryptoKey::from_settings(&cfg) else {
            panic!("expected aes128");
        };
        assert_eq!(k[15], 0x02);
        assert_eq!(&k[..15], &DEFAULT_PSK[..15]);
    }

    #[test]
    fn ctr_round_trip() {
        let key = CryptoKey::Aes128(DEFAULT_PSK);
        let plain = *b"hello meshtastic!!";
        let mut enc = plain;
        aes_ctr_inplace(&key, 0x1234_5678, 0x9abc_def0, &mut enc);
        assert_ne!(enc, plain);
        aes_ctr_inplace(&key, 0x1234_5678, 0x9abc_def0, &mut enc);
        assert_eq!(enc, plain);
    }
}
