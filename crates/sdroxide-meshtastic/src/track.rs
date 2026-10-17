//! Node table + message log merge for the Meshtastic panel snapshot.

use sdroxide_types::{MeshtasticMessage, MeshtasticNode, MeshtasticRegion, MeshtasticStatus};

const MAX_NODES: usize = 256;
const MAX_MESSAGES: usize = 500;

/// Side-effect-free description of what one packet should do to the tables.
#[derive(Debug, Clone)]
pub enum TrackerUpdate {
    Text {
        from: String,
        to: String,
        text: String,
        unix: i64,
        snr_db: Option<f32>,
    },
    NodeInfo {
        id: String,
        short_name: String,
        long_name: String,
        unix: i64,
        snr_db: Option<f32>,
        rssi_dbm: Option<f32>,
        hops: Option<u8>,
    },
    Position {
        id: String,
        lat: Option<f64>,
        lon: Option<f64>,
        unix: i64,
        snr_db: Option<f32>,
        rssi_dbm: Option<f32>,
        hops: Option<u8>,
    },
    Telemetry {
        id: String,
        battery_pct: Option<u8>,
        unix: i64,
        snr_db: Option<f32>,
        rssi_dbm: Option<f32>,
        hops: Option<u8>,
    },
    Opaque {
        from: String,
        to: String,
        portnum: u32,
        raw_hex: String,
        unix: i64,
        snr_db: Option<f32>,
    },
}

/// Touch metadata shared by every packet from a node.
#[derive(Debug, Clone)]
pub struct HeardPacket {
    pub id: String,
    pub unix: i64,
    pub snr_db: Option<f32>,
    pub rssi_dbm: Option<f32>,
    pub hops: Option<u8>,
}

/// Rolling node / message state owned by the worker.
#[derive(Debug, Default)]
pub struct Tracker {
    nodes: Vec<MeshtasticNode>,
    messages: Vec<MeshtasticMessage>,
    last_snr_db: Option<f32>,
    region: MeshtasticRegion,
}

impl Tracker {
    pub fn new(region: MeshtasticRegion) -> Self {
        Self {
            region,
            ..Default::default()
        }
    }

    pub fn set_region(&mut self, region: MeshtasticRegion) {
        self.region = region;
    }

    pub fn apply(&mut self, update: TrackerUpdate) {
        match update {
            TrackerUpdate::Text {
                from,
                to,
                text,
                unix,
                snr_db,
            } => {
                self.touch(&from, unix, snr_db, None, None);
                self.push_msg(MeshtasticMessage {
                    unix,
                    from,
                    to,
                    text,
                    raw: String::new(),
                    snr_db,
                });
                self.last_snr_db = snr_db.or(self.last_snr_db);
            }
            TrackerUpdate::NodeInfo {
                id,
                short_name,
                long_name,
                unix,
                snr_db,
                rssi_dbm,
                hops,
            } => {
                let n = self.touch(&id, unix, snr_db, rssi_dbm, hops);
                if !short_name.is_empty() {
                    n.short_name = short_name;
                }
                if !long_name.is_empty() {
                    n.long_name = long_name;
                }
                self.last_snr_db = snr_db.or(self.last_snr_db);
            }
            TrackerUpdate::Position {
                id,
                lat,
                lon,
                unix,
                snr_db,
                rssi_dbm,
                hops,
            } => {
                let n = self.touch(&id, unix, snr_db, rssi_dbm, hops);
                if lat.is_some() {
                    n.lat = lat;
                }
                if lon.is_some() {
                    n.lon = lon;
                }
                self.last_snr_db = snr_db.or(self.last_snr_db);
            }
            TrackerUpdate::Telemetry {
                id,
                battery_pct,
                unix,
                snr_db,
                rssi_dbm,
                hops,
            } => {
                let n = self.touch(&id, unix, snr_db, rssi_dbm, hops);
                if battery_pct.is_some() {
                    n.battery_pct = battery_pct;
                }
                self.last_snr_db = snr_db.or(self.last_snr_db);
            }
            TrackerUpdate::Opaque {
                from,
                to,
                portnum,
                raw_hex,
                unix,
                snr_db,
            } => {
                self.touch(&from, unix, snr_db, None, None);
                self.push_msg(MeshtasticMessage {
                    unix,
                    from,
                    to,
                    text: String::new(),
                    raw: format!("port {portnum}: {raw_hex}"),
                    snr_db,
                });
                self.last_snr_db = snr_db.or(self.last_snr_db);
            }
        }
        self.sort_nodes();
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }

    pub fn status(&self, listening: bool, unavailable: Option<String>) -> MeshtasticStatus {
        MeshtasticStatus {
            nodes: self.nodes.clone(),
            messages: self.messages.clone(),
            region: self.region,
            listening,
            sf: Some(11),
            bw_hz: Some(250_000),
            cr: Some(5),
            channel_name: "LongFast".into(),
            last_snr_db: self.last_snr_db,
            unavailable,
        }
    }

    fn touch(
        &mut self,
        id: &str,
        unix: i64,
        snr_db: Option<f32>,
        rssi_dbm: Option<f32>,
        hops: Option<u8>,
    ) -> &mut MeshtasticNode {
        if let Some(i) = self.nodes.iter().position(|n| n.id == id) {
            let n = &mut self.nodes[i];
            n.last_heard_unix = unix;
            if snr_db.is_some() {
                n.snr_db = snr_db;
            }
            if rssi_dbm.is_some() {
                n.rssi_dbm = rssi_dbm;
            }
            if hops.is_some() {
                n.hops = hops;
            }
            return &mut self.nodes[i];
        }
        self.nodes.push(MeshtasticNode {
            id: id.to_string(),
            short_name: String::new(),
            long_name: String::new(),
            last_heard_unix: unix,
            snr_db,
            rssi_dbm,
            hops,
            battery_pct: None,
            lat: None,
            lon: None,
        });
        if self.nodes.len() > MAX_NODES {
            // Drop the oldest-heard entry.
            if let Some((i, _)) = self
                .nodes
                .iter()
                .enumerate()
                .min_by_key(|(_, n)| n.last_heard_unix)
            {
                self.nodes.swap_remove(i);
            }
        }
        let last = self.nodes.len() - 1;
        &mut self.nodes[last]
    }

    fn push_msg(&mut self, msg: MeshtasticMessage) {
        self.messages.push(msg);
        if self.messages.len() > MAX_MESSAGES {
            let drop = self.messages.len() - MAX_MESSAGES;
            self.messages.drain(0..drop);
        }
    }

    fn sort_nodes(&mut self) {
        self.nodes.sort_by(|a, b| b.last_heard_unix.cmp(&a.last_heard_unix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_creates_node_and_message() {
        let mut t = Tracker::new(MeshtasticRegion::Eu868);
        t.apply(TrackerUpdate::Text {
            from: "!aabbccdd".into(),
            to: String::new(),
            text: "hi".into(),
            unix: 100,
            snr_db: Some(5.0),
        });
        assert_eq!(t.nodes.len(), 1);
        assert_eq!(t.messages.len(), 1);
        assert_eq!(t.nodes[0].id, "!aabbccdd");
        assert_eq!(t.messages[0].text, "hi");
    }

    #[test]
    fn nodeinfo_fills_names() {
        let mut t = Tracker::new(MeshtasticRegion::Us915);
        t.apply(TrackerUpdate::NodeInfo {
            id: "!aabbccdd".into(),
            short_name: "AB".into(),
            long_name: "Alpha Bravo".into(),
            unix: 200,
            snr_db: None,
            rssi_dbm: None,
            hops: Some(1),
        });
        assert_eq!(t.nodes[0].short_name, "AB");
        assert_eq!(t.nodes[0].long_name, "Alpha Bravo");
        assert_eq!(t.nodes[0].hops, Some(1));
    }
}
