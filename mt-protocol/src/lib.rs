//! # mt-protocol
//!
//! Pure, I/O free implementation of the Meshtastic client/radio wire protocol:
//!
//! - [`constants`] - well-known UUIDs, ports, addresses and handshake ids.
//! - [`contact`] - shared-contact links (QR/NFC/URL) built on `SharedContact`.
//! - [`frame`] - the `0x94 0xC3` stream frame codec used over TCP and serial
//!   (BLE exchanges protobufs directly over GATT characteristics).
//! - [`builders`] - helpers for constructing `ToRadio` messages (handshake,
//!   heartbeats, text messages, admin requests, ...).
//!
//! This crate deliberately has no async runtime or I/O dependencies so it can
//! be used from any context and tested exhaustively.

pub mod builders;
pub mod constants;
pub mod contact;
pub mod frame;

pub use frame::{DecodeEvent, FrameDecoder, FrameEncoder};

/// Re-exported so downstream crates always share one protobuf version.
pub use meshtastic_protobufs;
pub use prost;

/// The version of the bundled `meshtastic_protobufs` crate.
///
/// Kept as a constant so the UI can report which protocol revision it speaks.
/// Update it alongside the dependency in the workspace `Cargo.toml`.
pub const PROTOBUF_VERSION: &str = "2.7.8";
