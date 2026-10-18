//! MQTT decode-publish configuration (`mqtt.json`).
//!
//! sdroxide *feeds* an MQTT broker with decoded traffic — digi, WSPR, sonde,
//! Meshtastic — the same posture as [`crate::WsjtxConfig`]: output only, owned
//! by the station engine, edited on Settings → Servers. Nothing arriving on
//! the broker can touch the radio.
//!
//! Each decode kind has its own topic string. Defaults are `sdroxide/DIGI`,
//! `sdroxide/FT8`, `sdroxide/WSPR`, `sdroxide/SONDE`, and `sdroxide/MESH`. An
//! empty topic means that kind does not publish, even while MQTT is enabled.

use serde::{Deserialize, Serialize};

/// Topic for one decode stream. Empty `topic` disables that kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MqttTopic {
    pub topic: String,
}

impl MqttTopic {
    pub fn active(&self) -> bool {
        !self.topic.trim().is_empty()
    }

    pub fn trimmed(&self) -> Option<&str> {
        let t = self.topic.trim();
        if t.is_empty() { None } else { Some(t) }
    }
}

/// MQTT broker + per-kind topics (`mqtt.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MqttConfig {
    /// Off by default: publishing where the station is and what it hears is
    /// the operator's decision.
    pub enabled: bool,
    /// Broker host name or IP.
    pub host: String,
    /// 1883 plain MQTT, 8883 typical for TLS.
    pub port: u16,
    pub tls: bool,
    pub user: String,
    pub password: String,
    /// Empty → the engine picks `sdroxide-<callsign>`.
    pub client_id: String,
    /// 0 or 1 (capped at 1 by the publisher).
    pub qos: u8,
    /// All slotted digi decodes (FT8, FT4, JS8, …).
    pub digi: MqttTopic,
    pub wspr: MqttTopic,
    /// Radiosonde / RS41 packages.
    pub sonde: MqttTopic,
    /// Meshtastic mesh messages.
    pub mesh: MqttTopic,
    /// FT8-only stream (same payload as digi, only while the radio is in FT8).
    /// Appended last for postcard layout of [`crate::StationConfig`].
    #[serde(default)]
    pub ft8: MqttTopic,
}

impl Default for MqttConfig {
    fn default() -> Self {
        MqttConfig {
            enabled: false,
            host: String::new(),
            port: 1883,
            tls: false,
            user: String::new(),
            password: String::new(),
            client_id: String::new(),
            qos: 0,
            // Default under `sdroxide/` — clear a topic to disable that kind.
            digi: MqttTopic { topic: "sdroxide/DIGI".into() },
            wspr: MqttTopic { topic: "sdroxide/WSPR".into() },
            sonde: MqttTopic { topic: "sdroxide/SONDE".into() },
            mesh: MqttTopic { topic: "sdroxide/MESH".into() },
            ft8: MqttTopic { topic: "sdroxide/FT8".into() },
        }
    }
}

impl MqttConfig {
    pub fn broker_configured(&self) -> bool {
        self.enabled && !self.host.trim().is_empty() && self.port > 0
    }
}
