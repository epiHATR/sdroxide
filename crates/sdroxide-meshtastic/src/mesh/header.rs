//! 16-byte Meshtastic air header (`PacketHeader` in firmware `MeshPacket.h`).

/// Hop / flag bits packed into `flags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketFlags(pub u8);

impl PacketFlags {
    pub fn hop_limit(self) -> u8 {
        self.0 & 0x07
    }

    pub fn want_ack(self) -> bool {
        self.0 & 0x08 != 0
    }

    pub fn via_mqtt(self) -> bool {
        self.0 & 0x10 != 0
    }

    pub fn hop_start(self) -> u8 {
        (self.0 >> 5) & 0x07
    }
}

/// Decoded 16-byte packet header that rides ahead of the encrypted protobuf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketHeader {
    pub to: u32,
    pub from: u32,
    pub id: u32,
    pub flags: PacketFlags,
    pub channel: u8,
    pub next_hop: u8,
    pub relay_node: u8,
}

impl PacketHeader {
    pub const LEN: usize = 16;

    /// Parse little-endian header bytes. Returns `None` if shorter than 16.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < Self::LEN {
            return None;
        }
        Some(Self {
            to: u32::from_le_bytes(bytes[0..4].try_into().ok()?),
            from: u32::from_le_bytes(bytes[4..8].try_into().ok()?),
            id: u32::from_le_bytes(bytes[8..12].try_into().ok()?),
            flags: PacketFlags(bytes[12]),
            channel: bytes[13],
            next_hop: bytes[14],
            relay_node: bytes[15],
        })
    }

    /// Broadcast destination (`NODENUM_BROADCAST` / `0xffffffff`).
    pub fn is_broadcast(self) -> bool {
        self.to == 0xffff_ffff
    }

    /// Format a node number the way Meshtastic prints it (`!a1b2c3d4`).
    pub fn format_node(n: u32) -> String {
        if n == 0xffff_ffff {
            String::new()
        } else {
            format!("!{n:08x}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trip_layout() {
        let mut raw = [0u8; 16];
        raw[0..4].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        raw[4..8].copy_from_slice(&0x1122_3344u32.to_le_bytes());
        raw[8..12].copy_from_slice(&0x5566_7788u32.to_le_bytes());
        raw[12] = 0b1011_0011; // hop_start=5, via_mqtt, hop_limit=3
        raw[13] = 8; // LongFast default channel hash
        raw[14] = 0;
        raw[15] = 0;
        let h = PacketHeader::parse(&raw).unwrap();
        assert!(h.is_broadcast());
        assert_eq!(h.from, 0x1122_3344);
        assert_eq!(h.id, 0x5566_7788);
        assert_eq!(h.flags.hop_limit(), 3);
        assert!(h.flags.via_mqtt());
        assert_eq!(h.flags.hop_start(), 5);
        assert_eq!(h.channel, 8);
        assert_eq!(PacketHeader::format_node(h.from), "!11223344");
    }
}
