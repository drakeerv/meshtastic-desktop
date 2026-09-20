//! Helpers for constructing `ToRadio` messages without touching raw
//! protobuf field plumbing at every call site.
//!
//! Mirrors message construction in the official clients
//! (`Meshtastic-Android`, `Meshtastic-Apple`, `Meshtastic-JS`).

use meshtastic_protobufs::meshtastic::{
    AdminMessage, Data, MeshPacket, PortNum, ToRadio, User, admin_message,
    admin_message::{ConfigType, ModuleConfigType},
    mesh_packet, to_radio,
};

use crate::constants::{BROADCAST_ADDR, DEFAULT_HOP_LIMIT};

/// Builds a `want_config_id` handshake message.
pub fn want_config(nonce: u32) -> ToRadio {
    ToRadio {
        payload_variant: Some(to_radio::PayloadVariant::WantConfigId(nonce)),
    }
}

/// Builds the Stage 1 handshake request (nonce `69420`): configuration,
/// channels and the file manifest, but no node database.
pub fn initial_handshake() -> ToRadio {
    want_config(crate::constants::HANDSHAKE_NONCE_1)
}

/// Builds the Stage 2 handshake request (nonce `69421`): the device's stored
/// node database.
pub fn second_handshake() -> ToRadio {
    want_config(crate::constants::HANDSHAKE_NONCE_2)
}

/// Builds an empty heartbeat. Keeps the connection alive.
pub fn heartbeat() -> ToRadio {
    ToRadio {
        payload_variant: Some(to_radio::PayloadVariant::Heartbeat(
            meshtastic_protobufs::meshtastic::Heartbeat {},
        )),
    }
}

/// Builds a polite disconnect notification (optional for clients).
pub fn disconnect() -> ToRadio {
    ToRadio {
        payload_variant: Some(to_radio::PayloadVariant::Disconnect(true)),
    }
}

/// Sets the device's clock from the host clock (Unix seconds).
///
/// The official clients send this early in the handshake. Radios with no
/// GPS or RTC leave `rx_time` at zero on received packets, and the firmware
/// then skips `last_heard` updates entirely, so node timestamps go stale
/// across reconnects. Sent to the local node with zero hops and no session
/// passkey (the firmware skips passkey checks for client packets).
pub fn set_time_only(seconds: u32) -> ToRadio {
    admin_packet(
        BROADCAST_ADDR,
        admin(admin_message::PayloadVariant::SetTimeOnly(seconds)),
        false,
    )
}

/// Options for building a text message.
#[derive(Debug, Clone)]
pub struct TextMessage {
    /// Destination node number (`BROADCAST_ADDR` for channel-wide).
    pub to: u32,
    /// Channel index (0-7; PKI direct messages use index 8).
    pub channel: u32,
    pub text: String,
    /// Request acknowledgement / routing reports.
    pub want_ack: bool,
    /// If set, this is a reply to the given packet id (emoji reaction).
    pub reply_id: Option<u32>,
}

impl TextMessage {
    /// A plain message to a channel's broadcast address.
    pub fn broadcast(channel: u32, text: impl Into<String>) -> Self {
        Self {
            to: BROADCAST_ADDR,
            channel,
            text: text.into(),
            want_ack: true,
            reply_id: None,
        }
    }

    /// A direct message to a specific node.
    pub fn direct(to: u32, text: impl Into<String>) -> Self {
        Self {
            to,
            channel: crate::constants::PKI_CHANNEL_INDEX as u32,
            text: text.into(),
            want_ack: true,
            reply_id: None,
        }
    }

    /// A reply carrying only an emoji (a reaction to `reply_id`).
    pub fn reaction(to: u32, channel: u32, emoji: impl Into<String>, reply_id: u32) -> Self {
        Self {
            to,
            channel,
            text: emoji.into(),
            want_ack: false,
            reply_id: Some(reply_id),
        }
    }
}

/// Builds a `ToRadio` carrying a text message [`MeshPacket`].
pub fn text_message(msg: TextMessage) -> ToRadio {
    packet(MeshPacket {
        to: msg.to,
        channel: msg.channel,
        want_ack: msg.want_ack,
        id: next_packet_id(),
        hop_limit: DEFAULT_HOP_LIMIT,
        payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
            portnum: PortNum::TextMessageApp as i32,
            payload: msg.text.into_bytes(),
            want_response: false,
            reply_id: msg.reply_id.unwrap_or(0),
            ..Default::default()
        })),
        ..Default::default()
    })
}

/// Wraps a [`MeshPacket`] into a `ToRadio`.
pub fn packet(p: MeshPacket) -> ToRadio {
    ToRadio {
        payload_variant: Some(to_radio::PayloadVariant::Packet(p)),
    }
}

/// Builds an admin message packet addressed to a node.
///
/// Admin packets to the local node (via BLE/serial/TCP) travel with the
/// broadcast destination and zero hops; remote admin goes to the node
/// number on the primary channel with normal hop limits.
pub fn admin_packet(dest: u32, admin: AdminMessage, want_response: bool) -> ToRadio {
    packet(MeshPacket {
        to: dest,
        channel: 0,
        want_ack: false,
        id: next_packet_id(),
        hop_limit: if dest == BROADCAST_ADDR {
            0
        } else {
            DEFAULT_HOP_LIMIT
        },
        payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
            portnum: PortNum::AdminApp as i32,
            payload: crate::frame::encode_protobuf(&admin),
            want_response,
            ..Default::default()
        })),
        ..Default::default()
    })
}

/// Builds an [`AdminMessage`] from a variant.
pub fn admin(variant: admin_message::PayloadVariant) -> AdminMessage {
    AdminMessage {
        payload_variant: Some(variant),
        ..Default::default()
    }
}

/// Requests one config section (local interface).
pub fn get_config_request(config_type: ConfigType) -> ToRadio {
    admin_packet(
        BROADCAST_ADDR,
        admin(admin_message::PayloadVariant::GetConfigRequest(
            config_type as i32,
        )),
        true,
    )
}

/// Requests one module config section.
pub fn get_module_config_request(config_type: ModuleConfigType) -> ToRadio {
    admin_packet(
        BROADCAST_ADDR,
        admin(admin_message::PayloadVariant::GetModuleConfigRequest(
            config_type as i32,
        )),
        true,
    )
}

/// Requests a channel by index.
pub fn get_channel_request(index: u32) -> ToRadio {
    admin_packet(
        BROADCAST_ADDR,
        admin(admin_message::PayloadVariant::GetChannelRequest(index)),
        true,
    )
}

/// Requests the owner/user of the local node.
pub fn get_owner_request() -> ToRadio {
    admin_packet(
        BROADCAST_ADDR,
        admin(admin_message::PayloadVariant::GetOwnerRequest(true)),
        true,
    )
}

/// Requests device metadata (firmware version, capabilities).
pub fn get_device_metadata_request(dest: u32) -> ToRadio {
    admin_packet(
        dest,
        admin(admin_message::PayloadVariant::GetDeviceMetadataRequest(
            true,
        )),
        true,
    )
}

/// Sets the owner (long/short name) of the local node.
pub fn set_owner(
    long_name: impl Into<String>,
    short_name: impl Into<String>,
    is_licensed: bool,
) -> ToRadio {
    admin_packet(
        BROADCAST_ADDR,
        admin(admin_message::PayloadVariant::SetOwner(User {
            long_name: long_name.into(),
            short_name: short_name.into(),
            is_licensed,
            ..Default::default()
        })),
        false,
    )
}

/// Begins an edit transaction (firmware >= 2.5 defers persist + reboot
/// until commit).
pub fn begin_edit_settings() -> ToRadio {
    admin_packet(
        BROADCAST_ADDR,
        admin(admin_message::PayloadVariant::BeginEditSettings(true)),
        false,
    )
}

/// Commits an edit transaction, persisting changes.
pub fn commit_edit_settings() -> ToRadio {
    admin_packet(
        BROADCAST_ADDR,
        admin(admin_message::PayloadVariant::CommitEditSettings(true)),
        false,
    )
}

/// Reboots a node after the given delay in seconds.
pub fn reboot_seconds(dest: u32, seconds: i32) -> ToRadio {
    admin_packet(
        dest,
        admin(admin_message::PayloadVariant::RebootSeconds(seconds)),
        false,
    )
}

/// Shuts a node down after the given delay in seconds.
pub fn shutdown_seconds(dest: u32, seconds: i32) -> ToRadio {
    admin_packet(
        dest,
        admin(admin_message::PayloadVariant::ShutdownSeconds(seconds)),
        false,
    )
}

/// Factory resets a node (config only, or the whole device).
pub fn factory_reset(dest: u32, full_device: bool) -> ToRadio {
    let variant = if full_device {
        admin_message::PayloadVariant::FactoryResetDevice(1)
    } else {
        admin_message::PayloadVariant::FactoryResetConfig(1)
    };
    admin_packet(dest, admin(variant), false)
}

/// Requests a position report from a node.
pub fn position_request(dest: u32) -> ToRadio {
    packet(MeshPacket {
        to: dest,
        channel: 0,
        want_ack: false,
        id: next_packet_id(),
        hop_limit: DEFAULT_HOP_LIMIT,
        payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
            portnum: PortNum::PositionApp as i32,
            payload: Vec::new(),
            want_response: true,
            ..Default::default()
        })),
        ..Default::default()
    })
}

/// Requests a traceroute to a node.
pub fn traceroute_request(dest: u32) -> ToRadio {
    packet(MeshPacket {
        to: dest,
        channel: 0,
        want_ack: false,
        id: next_packet_id(),
        hop_limit: DEFAULT_HOP_LIMIT,
        payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
            portnum: PortNum::TracerouteApp as i32,
            payload: Vec::new(),
            want_response: true,
            ..Default::default()
        })),
        ..Default::default()
    })
}

/// Generates a random, non-zero packet id.
///
/// Ids only need to be unique per sending node; the firmware treats 0 as
/// "no id", so we always set the low bit.
pub fn next_packet_id() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970");
    let mut x = now.as_nanos() as u64 ^ (std::process::id() as u64) << 32 ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    (x as u32) | 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;

    #[test]
    fn handshake_uses_official_nonces() {
        let m = initial_handshake();
        match m.payload_variant {
            Some(to_radio::PayloadVariant::WantConfigId(n)) => assert_eq!(n, 69_420),
            other => panic!("unexpected variant: {other:?}"),
        }
        let m = second_handshake();
        match m.payload_variant {
            Some(to_radio::PayloadVariant::WantConfigId(n)) => assert_eq!(n, 69_421),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn text_message_shape() {
        let m = text_message(TextMessage::broadcast(0, "hello"));
        let Some(to_radio::PayloadVariant::Packet(p)) = m.payload_variant else {
            panic!("no packet");
        };
        assert_eq!(p.to, BROADCAST_ADDR);
        assert_eq!(p.channel, 0);
        assert!(p.want_ack);
        assert_ne!(p.id, 0);
        let Some(mesh_packet::PayloadVariant::Decoded(d)) = p.payload_variant else {
            panic!("not decoded");
        };
        assert_eq!(d.portnum, PortNum::TextMessageApp as i32);
        assert_eq!(d.payload, b"hello");
    }

    #[test]
    fn direct_message_uses_pki_channel() {
        let m = text_message(TextMessage::direct(0x1234_5678, "hi"));
        let Some(to_radio::PayloadVariant::Packet(p)) = m.payload_variant else {
            panic!("no packet");
        };
        assert_eq!(p.channel, crate::constants::PKI_CHANNEL_INDEX as u32);
        assert_eq!(p.to, 0x1234_5678);
    }

    #[test]
    fn reaction_sets_reply_id() {
        let m = text_message(TextMessage::reaction(BROADCAST_ADDR, 0, "👍", 42));
        let Some(to_radio::PayloadVariant::Packet(p)) = m.payload_variant else {
            panic!("no packet");
        };
        let Some(mesh_packet::PayloadVariant::Decoded(d)) = p.payload_variant else {
            panic!("not decoded");
        };
        assert_eq!(d.reply_id, 42);
        assert_eq!(d.payload, "👍".as_bytes());
    }

    #[test]
    fn admin_messages_encode() {
        // Round trip an admin request to ensure the payload wire format is
        // a valid AdminMessage.
        let m = get_owner_request();
        let Some(to_radio::PayloadVariant::Packet(p)) = m.payload_variant else {
            panic!("no packet");
        };
        let Some(mesh_packet::PayloadVariant::Decoded(d)) = p.payload_variant else {
            panic!("not decoded");
        };
        assert_eq!(d.portnum, PortNum::AdminApp as i32);
        assert!(d.want_response);
        let admin = AdminMessage::decode(d.payload.as_slice()).unwrap();
        assert!(matches!(
            admin.payload_variant,
            Some(admin_message::PayloadVariant::GetOwnerRequest(true))
        ));
    }

    #[test]
    fn local_admin_uses_zero_hops_remote_uses_default() {
        let local = get_owner_request();
        let Some(to_radio::PayloadVariant::Packet(p)) = local.payload_variant else {
            panic!("no packet");
        };
        assert_eq!(p.hop_limit, 0);
        assert_eq!(p.to, BROADCAST_ADDR);

        let remote = reboot_seconds(0xdead_beef, 5);
        let Some(to_radio::PayloadVariant::Packet(p)) = remote.payload_variant else {
            panic!("no packet");
        };
        assert_eq!(p.hop_limit, DEFAULT_HOP_LIMIT);
        assert_eq!(p.to, 0xdead_beef);
    }

    #[test]
    fn set_time_only_is_a_local_admin_packet() {
        let m = set_time_only(1_700_000_000);
        let Some(to_radio::PayloadVariant::Packet(p)) = m.payload_variant else {
            panic!("no packet");
        };
        assert_eq!(p.to, BROADCAST_ADDR);
        assert_eq!(p.hop_limit, 0);
        let Some(mesh_packet::PayloadVariant::Decoded(d)) = p.payload_variant else {
            panic!("not decoded");
        };
        let admin = AdminMessage::decode(d.payload.as_slice()).unwrap();
        assert!(matches!(
            admin.payload_variant,
            Some(admin_message::PayloadVariant::SetTimeOnly(1_700_000_000))
        ));
    }

    #[test]
    fn packet_ids_are_nonzero_and_vary() {
        let a = next_packet_id();
        let b = next_packet_id();
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b);
    }
}
