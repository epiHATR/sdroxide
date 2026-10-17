//! Meshtastic mesh decode types — shared by the native engine, the wire
//! protocol and the UI (native + WASM).
//!
//! First ship is RX-only: a node table, a message log and a map of positions
//! when a packet carries them. The LoRa modem itself is a later lane; these
//! structs are what that lane will fill.
//!
//! Region codes and band edges follow Meshtastic firmware `RegionInfo`
//! (`RadioInterface.cpp`). Dial centres are the LongFast default frequency
//! slot (channel name `"LongFast"` hashed the same way the firmware does).

use serde::{Deserialize, Serialize};

/// EU_868 LongFast centre (Hz) — single slot at mid-band.
pub const MESHTASTIC_EU868_HZ: f64 = 869_525_000.0;
/// US LongFast default slot 20 centre (Hz).
pub const MESHTASTIC_US915_HZ: f64 = 906_875_000.0;
/// ANZ_433 LongFast default centre (Hz) — see [`MeshtasticRegion::Asia433`].
pub const MESHTASTIC_ASIA433_HZ: f64 = 434_425_000.0;

/// LongFast modem bandwidth used for default slot centres (Hz).
const LONG_FAST_BW_HZ: f64 = 250_000.0;

/// Regulatory / frequency plan the operator is listening on.
///
/// Append-only: postcard and serde both care about variant order. The first
/// three stay put for wire compatibility; later variants match Meshtastic
/// `Config.LoRa.RegionCode` names (there is no `AS_923` in Meshtastic — use
/// [`MeshtasticRegion::Sg923`] / [`MeshtasticRegion::Jp`] / … for that band).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum MeshtasticRegion {
    #[default]
    Eu868 = 0,
    Us915 = 1,
    /// ANZ 433 MHz (`ANZ_433`). Kept as `Asia433` for the v186 wire name.
    Asia433 = 2,
    // ── appended ──────────────────────────────────────────────────────────
    Eu433 = 3,
    Cn = 4,
    Jp = 5,
    Anz = 6,
    Kr = 7,
    Tw = 8,
    Ru = 9,
    In = 10,
    Nz865 = 11,
    Th = 12,
    Ua433 = 13,
    Ua868 = 14,
    My433 = 15,
    My919 = 16,
    Sg923 = 17,
    Ph433 = 18,
    Ph868 = 19,
    Ph915 = 20,
    Kz433 = 21,
    Kz863 = 22,
    Np865 = 23,
    Br902 = 24,
    Lora24 = 25,
}

impl MeshtasticRegion {
    /// Every region offered in the picker, in a stable display order
    /// (common ISM first, then by Meshtastic code name).
    pub const ALL: [MeshtasticRegion; 26] = [
        MeshtasticRegion::Eu868,
        MeshtasticRegion::Us915,
        MeshtasticRegion::Anz,
        MeshtasticRegion::Sg923,
        MeshtasticRegion::Jp,
        MeshtasticRegion::Kr,
        MeshtasticRegion::Tw,
        MeshtasticRegion::Th,
        MeshtasticRegion::My919,
        MeshtasticRegion::Cn,
        MeshtasticRegion::In,
        MeshtasticRegion::Ru,
        MeshtasticRegion::Nz865,
        MeshtasticRegion::Np865,
        MeshtasticRegion::Kz863,
        MeshtasticRegion::Br902,
        MeshtasticRegion::Ph915,
        MeshtasticRegion::Ph868,
        MeshtasticRegion::Ua868,
        MeshtasticRegion::Eu433,
        MeshtasticRegion::Asia433,
        MeshtasticRegion::My433,
        MeshtasticRegion::Ua433,
        MeshtasticRegion::Ph433,
        MeshtasticRegion::Kz433,
        MeshtasticRegion::Lora24,
    ];

    /// Chip / combo label (Meshtastic region code style).
    pub fn label(self) -> &'static str {
        match self {
            MeshtasticRegion::Eu868 => "EU_868",
            MeshtasticRegion::Us915 => "US",
            MeshtasticRegion::Asia433 => "ANZ_433",
            MeshtasticRegion::Eu433 => "EU_433",
            MeshtasticRegion::Cn => "CN",
            MeshtasticRegion::Jp => "JP",
            MeshtasticRegion::Anz => "ANZ",
            MeshtasticRegion::Kr => "KR",
            MeshtasticRegion::Tw => "TW",
            MeshtasticRegion::Ru => "RU",
            MeshtasticRegion::In => "IN",
            MeshtasticRegion::Nz865 => "NZ_865",
            MeshtasticRegion::Th => "TH",
            MeshtasticRegion::Ua433 => "UA_433",
            MeshtasticRegion::Ua868 => "UA_868",
            MeshtasticRegion::My433 => "MY_433",
            MeshtasticRegion::My919 => "MY_919",
            MeshtasticRegion::Sg923 => "SG_923",
            MeshtasticRegion::Ph433 => "PH_433",
            MeshtasticRegion::Ph868 => "PH_868",
            MeshtasticRegion::Ph915 => "PH_915",
            MeshtasticRegion::Kz433 => "KZ_433",
            MeshtasticRegion::Kz863 => "KZ_863",
            MeshtasticRegion::Np865 => "NP_865",
            MeshtasticRegion::Br902 => "BR_902",
            MeshtasticRegion::Lora24 => "LORA_24",
        }
    }

    /// Short description for hover / setup.
    pub fn description(self) -> &'static str {
        match self {
            MeshtasticRegion::Eu868 => "European Union 868 MHz",
            MeshtasticRegion::Us915 => "United States 902–928 MHz",
            MeshtasticRegion::Asia433 => "Australia / New Zealand 433 MHz",
            MeshtasticRegion::Eu433 => "European Union 433 MHz",
            MeshtasticRegion::Cn => "China 470–510 MHz",
            MeshtasticRegion::Jp => "Japan 920.5–923.5 MHz (AS923 family)",
            MeshtasticRegion::Anz => "Australia / New Zealand 915–928 MHz",
            MeshtasticRegion::Kr => "Korea 920–923 MHz (AS923 family)",
            MeshtasticRegion::Tw => "Taiwan 920–925 MHz (AS923 family)",
            MeshtasticRegion::Ru => "Russia 868.7–869.2 MHz",
            MeshtasticRegion::In => "India 865–867 MHz",
            MeshtasticRegion::Nz865 => "New Zealand 864–868 MHz",
            MeshtasticRegion::Th => "Thailand 920–925 MHz (AS923 family)",
            MeshtasticRegion::Ua433 => "Ukraine 433 MHz",
            MeshtasticRegion::Ua868 => "Ukraine 868 MHz",
            MeshtasticRegion::My433 => "Malaysia 433 MHz",
            MeshtasticRegion::My919 => "Malaysia 919–924 MHz (AS923 family)",
            MeshtasticRegion::Sg923 => "Singapore 917–925 MHz (AS923 family)",
            MeshtasticRegion::Ph433 => "Philippines 433 MHz",
            MeshtasticRegion::Ph868 => "Philippines 868 MHz",
            MeshtasticRegion::Ph915 => "Philippines 915–918 MHz",
            MeshtasticRegion::Kz433 => "Kazakhstan 433 MHz",
            MeshtasticRegion::Kz863 => "Kazakhstan 863–868 MHz",
            MeshtasticRegion::Np865 => "Nepal 865–868 MHz",
            MeshtasticRegion::Br902 => "Brazil 902–907.5 MHz",
            MeshtasticRegion::Lora24 => "Worldwide 2.4 GHz",
        }
    }

    /// Band edges in MHz — Meshtastic `RegionInfo` freqStart / freqEnd.
    pub fn band_mhz(self) -> (f64, f64) {
        match self {
            MeshtasticRegion::Us915 => (902.0, 928.0),
            MeshtasticRegion::Eu433 => (433.0, 434.0),
            MeshtasticRegion::Eu868 => (869.4, 869.65),
            MeshtasticRegion::Cn => (470.0, 510.0),
            MeshtasticRegion::Jp => (920.5, 923.5),
            MeshtasticRegion::Anz => (915.0, 928.0),
            MeshtasticRegion::Asia433 => (433.05, 434.79),
            MeshtasticRegion::Ru => (868.7, 869.2),
            MeshtasticRegion::Kr => (920.0, 923.0),
            MeshtasticRegion::Tw => (920.0, 925.0),
            MeshtasticRegion::In => (865.0, 867.0),
            MeshtasticRegion::Nz865 => (864.0, 868.0),
            MeshtasticRegion::Th => (920.0, 925.0),
            MeshtasticRegion::Ua433 => (433.0, 434.7),
            MeshtasticRegion::Ua868 => (868.0, 868.6),
            MeshtasticRegion::My433 => (433.0, 435.0),
            MeshtasticRegion::My919 => (919.0, 924.0),
            MeshtasticRegion::Sg923 => (917.0, 925.0),
            MeshtasticRegion::Ph433 => (433.0, 434.7),
            MeshtasticRegion::Ph868 => (868.0, 869.4),
            MeshtasticRegion::Ph915 => (915.0, 918.0),
            MeshtasticRegion::Kz433 => (433.075, 434.775),
            MeshtasticRegion::Kz863 => (863.0, 868.0),
            MeshtasticRegion::Np865 => (865.0, 868.0),
            MeshtasticRegion::Br902 => (902.0, 907.5),
            MeshtasticRegion::Lora24 => (2400.0, 2483.5),
        }
    }

    /// Dial centre for this region when starting the mode or switching plan.
    ///
    /// LongFast default slot, matching firmware: `hash("LongFast") % n + 1`
    /// with `freq = start + bw/2 + (slot−1)·bw`.
    pub fn center_hz(self) -> f64 {
        let (start_mhz, end_mhz) = self.band_mhz();
        long_fast_center_hz(start_mhz, end_mhz)
    }

    /// True when `hz` sits inside this region's LoRa allocation (band edges
    /// inclusive). Used to clear the "out of band" panel warning once the
    /// dial is on a supported frequency.
    pub fn contains_hz(self, hz: f64) -> bool {
        let (lo_mhz, hi_mhz) = self.band_mhz();
        let lo = lo_mhz * 1e6;
        let hi = hi_mhz * 1e6;
        hz >= lo && hz <= hi
    }
}

/// Firmware djb2-style hash for a channel / preset display name.
fn meshtastic_name_hash(name: &str) -> u32 {
    let mut hash: u32 = 5381;
    for b in name.bytes() {
        hash = hash.wrapping_add(hash.wrapping_shl(5).wrapping_add(u32::from(b)));
    }
    hash
}

/// LongFast default centre frequency in Hz for a Meshtastic band.
fn long_fast_center_hz(freq_start_mhz: f64, freq_end_mhz: f64) -> f64 {
    let bw_mhz = LONG_FAST_BW_HZ / 1e6;
    let mut num = ((freq_end_mhz - freq_start_mhz) / bw_mhz).floor() as u32;
    if num == 0 {
        num = 1;
    }
    let slot0 = meshtastic_name_hash("LongFast") % num;
    let freq_mhz = freq_start_mhz + bw_mhz / 2.0 + f64::from(slot0) * bw_mhz;
    freq_mhz * 1e6
}

/// How the decoder expands a channel PSK (Meshtastic firmware rules).
///
/// Append-only for postcard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum MeshtasticPskKind {
    /// Public default LongFast key (`0x01` shorthand).
    #[default]
    Default = 0,
    /// `simpleN` — default key with last byte replaced by `N` (`1`..=`10`).
    Simple = 1,
    /// Explicit 16-byte AES-128 key (bytes in [`MeshtasticSettings::psk_bytes`]).
    Key16 = 2,
    /// Explicit 32-byte AES-256 key.
    Key32 = 3,
}

/// Target DDC lane rate for LongFast listen (Hz) — matches OpenWebRX+ / lorarx
/// (`ifRate` 1 Msps). Must be ≥ modem BW; 1 Msps gives headroom for SF11/250k.
pub const MESHTASTIC_LANE_RATE_HZ: f64 = 1_000_000.0;
/// Minimum front-end sample rate that can feed the Meshtastic DDC usefully.
pub const MESHTASTIC_MIN_RATE_HZ: f64 = 1_000_000.0;

/// Persisted Meshtastic listen settings on [`crate::RadioState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshtasticSettings {
    /// Region / frequency plan.
    #[serde(default)]
    pub region: MeshtasticRegion,
    /// Which PSK expansion to use.
    #[serde(default)]
    pub psk_kind: MeshtasticPskKind,
    /// For [`MeshtasticPskKind::Simple`]: `1`..=`10` (UI `simple1`…`simple10`).
    #[serde(default = "default_simple_n")]
    pub psk_simple: u8,
    /// Raw key bytes for Key16 / Key32 (unused bytes zero). Appended last.
    #[serde(default)]
    pub psk_bytes: [u8; 32],
}

fn default_simple_n() -> u8 {
    1
}

impl Default for MeshtasticSettings {
    fn default() -> Self {
        Self {
            region: MeshtasticRegion::Eu868,
            psk_kind: MeshtasticPskKind::Default,
            psk_simple: 1,
            psk_bytes: [0; 32],
        }
    }
}

impl MeshtasticSettings {
    /// Clamp operator values into a legal range.
    pub fn sane(mut self) -> Self {
        if self.psk_simple < 1 {
            self.psk_simple = 1;
        }
        if self.psk_simple > 10 {
            self.psk_simple = 10;
        }
        self
    }
}

/// One mesh node heard on the air.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshtasticNode {
    /// Node number as printed by Meshtastic (hex, e.g. `!a1b2c3d4`).
    pub id: String,
    /// Short name when known; empty until a NodeInfo arrives.
    #[serde(default)]
    pub short_name: String,
    /// Long name when known.
    #[serde(default)]
    pub long_name: String,
    /// Unix time of the last packet from this node.
    pub last_heard_unix: i64,
    /// SNR of the last packet, dB. `None` until the modem reports one.
    #[serde(default)]
    pub snr_db: Option<f32>,
    /// RSSI of the last packet, dBm. `None` until the modem reports one.
    #[serde(default)]
    pub rssi_dbm: Option<f32>,
    /// Hop count of the last packet when known.
    #[serde(default)]
    pub hops: Option<u8>,
    /// Battery percent when a device metrics packet carried one.
    #[serde(default)]
    pub battery_pct: Option<u8>,
    /// Last known position, degrees. `None` until a position packet arrives.
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lon: Option<f64>,
}

/// One mesh text (or opaque) message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshtasticMessage {
    /// Unix time the packet was decoded.
    pub unix: i64,
    /// From node id (`!…`).
    pub from: String,
    /// To node id, or empty for broadcast.
    #[serde(default)]
    pub to: String,
    /// Decoded UTF-8 text when the payload was plaintext.
    #[serde(default)]
    pub text: String,
    /// Compact hex/raw when the payload was encrypted or not text.
    #[serde(default)]
    pub raw: String,
    /// SNR of this packet, dB, when known.
    #[serde(default)]
    pub snr_db: Option<f32>,
}

/// Live Meshtastic decode snapshot for the operating panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MeshtasticStatus {
    /// Nodes still on the list, most recently heard first.
    #[serde(default)]
    pub nodes: Vec<MeshtasticNode>,
    /// Rolling message log, newest last (panel scrolls to the end).
    #[serde(default)]
    pub messages: Vec<MeshtasticMessage>,
    /// Region the decoder believes it is on (mirrors settings once applied).
    #[serde(default)]
    pub region: MeshtasticRegion,
    /// True while the engine is in Meshtastic mode and accepting IQ.
    #[serde(default)]
    pub listening: bool,
    /// Spreading factor of the last decode, when known.
    #[serde(default)]
    pub sf: Option<u8>,
    /// Bandwidth Hz of the last decode, when known.
    #[serde(default)]
    pub bw_hz: Option<u32>,
    /// Coding rate denominator (e.g. 5 for 4/5), when known.
    #[serde(default)]
    pub cr: Option<u8>,
    /// Channel / modem preset name when known (e.g. `LongFast`).
    #[serde(default)]
    pub channel_name: String,
    /// SNR of the most recent packet, dB.
    #[serde(default)]
    pub last_snr_db: Option<f32>,
    /// Why the decoder is not running, when it is not (missing modem, etc.).
    #[serde(default)]
    pub unavailable: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_fast_centres_match_known_slots() {
        assert!((MeshtasticRegion::Us915.center_hz() - MESHTASTIC_US915_HZ).abs() < 1.0);
        assert!((MeshtasticRegion::Eu868.center_hz() - MESHTASTIC_EU868_HZ).abs() < 1.0);
        assert!((MeshtasticRegion::Asia433.center_hz() - MESHTASTIC_ASIA433_HZ).abs() < 1.0);
        // Singapore AS923-family LongFast default.
        assert!((MeshtasticRegion::Sg923.center_hz() - 917_875_000.0).abs() < 1.0);
    }

    #[test]
    fn region_discriminants_are_stable() {
        assert_eq!(MeshtasticRegion::Eu868 as u8, 0);
        assert_eq!(MeshtasticRegion::Us915 as u8, 1);
        assert_eq!(MeshtasticRegion::Asia433 as u8, 2);
        assert_eq!(MeshtasticRegion::Sg923 as u8, 17);
        assert_eq!(MeshtasticRegion::Lora24 as u8, 25);
    }

    #[test]
    fn contains_hz_matches_band_edges() {
        let r = MeshtasticRegion::Sg923;
        assert!(r.contains_hz(r.center_hz()));
        assert!(r.contains_hz(917e6));
        assert!(r.contains_hz(925e6));
        assert!(!r.contains_hz(906.875e6)); // US centre
        assert!(!r.contains_hz(2.4e9)); // LORA_24
    }
}
