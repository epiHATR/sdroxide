//! LoRa PHY for Meshtastic LongFast — soft demod (dev) + dxlAPRS `lorarx` (default).

pub mod demod;
pub mod fec;
pub mod lorarx;

pub use demod::{LongFastDemod, PhyPacket, BW_HZ, CR, DEM_RATE_HZ, SF, SYNC_WORD};
pub use lorarx::{LoraRxPhy, LORARX_MISSING};
