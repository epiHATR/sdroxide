//! Meshtastic LongFast RX: dxlAPRS `lorarx` PHY + mesh header / AES-CTR / portnums.
//!
//! Native-only — runs in the engine off a ~1 Msps DDC (OpenWebRX+ shape). The
//! UI paints [`sdroxide_types::MeshtasticStatus`] the worker already built.
//!
//! # Shape
//!
//! ```text
//! window I/Q ──► lorarx (stdin f32) ──► JSON payload ──► AES-CTR / portnums
//!                                                              │
//!                                                              ▼
//!                                                    MeshtasticStatus (~2 Hz)
//! ```
//!
//! Requires `lorarx` on `PATH`. Mesh crypto matches Meshtastic firmware
//! (`PacketHeader`, `CryptoEngine`, `defaultpsk`).

mod controller;
pub mod lora;
pub mod mesh;
pub mod track;

pub use controller::{MeshtasticAction, MeshtasticController};
pub use lora::{LoraRxPhy, LongFastDemod, PhyPacket, LORARX_MISSING};
pub use mesh::{CryptoKey, DEFAULT_PSK, PacketHeader, decode_airframe};
pub use track::{Tracker, TrackerUpdate};
