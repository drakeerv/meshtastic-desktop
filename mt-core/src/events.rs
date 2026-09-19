//! The public vocabulary of the core: commands in, events out.
//!
//! The UI never touches transports, protobuf plumbing or the database; it
//! dispatches [`CoreCommand`]s and renders [`CoreEvent`]s.

use std::time::Duration;

use meshtastic_protobufs::meshtastic::{
    Channel, Config, DeviceMetadata, ModuleConfig, MyNodeInfo, NodeInfo, Position, QueueStatus,
    SharedContact, Telemetry,
};
use mt_persistence::{MessageRecord, MessageStatus};
use mt_transport::DeviceAddress;

/// High level state of the connection, as shown in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// No device selected / fully idle.
    Disconnected,
    /// Opening the link.
    Connecting(DeviceAddress),
    /// Link is up, config handshake in flight.
    Handshaking(DeviceAddress),
    /// Handshake complete; the mesh is live.
    Connected {
        address: DeviceAddress,
        node_num: u32,
    },
    /// Waiting before another attempt.
    Reconnecting {
        address: DeviceAddress,
        attempt: u32,
        delay: Duration,
    },
    /// A graceful disconnect is in progress.
    Disconnecting(DeviceAddress),
}

impl ConnectionState {
    /// Whether the mesh is currently usable.
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnectionState::Connected { .. })
    }

    /// The address this state refers to, if any.
    pub fn address(&self) -> Option<&DeviceAddress> {
        match self {
            ConnectionState::Disconnected => None,
            ConnectionState::Connecting(a)
            | ConnectionState::Handshaking(a)
            | ConnectionState::Disconnecting(a) => Some(a),
            ConnectionState::Connected { address, .. }
            | ConnectionState::Reconnecting { address, .. } => Some(address),
        }
    }
}

/// Everything the core reports to the UI. Cloned into a broadcast channel,
/// so variants carry owned data.
#[derive(Debug, Clone)]
pub enum CoreEvent {
    /// Connection lifecycle transition.
    Connection(ConnectionState),
    /// The connected node's own identity/number.
    MyInfo(Box<MyNodeInfo>),
    /// Device metadata (firmware version, capabilities).
    Metadata(Box<DeviceMetadata>),
    /// A device config section.
    Config(Box<Config>),
    /// A module config section.
    ModuleConfig(Box<ModuleConfig>),
    /// A channel slot.
    Channel(Box<Channel>),
    /// A node was created or updated.
    Node(Box<NodeInfo>),
    /// A node was removed.
    NodeRemoved(u32),
    /// A node's signal strength (RSSI) was observed while hearing it.
    Rssi { node_num: u32, rssi: i32 },
    /// Full message history snapshot (sent after connecting).
    MessagesLoaded(Vec<MessageRecord>),
    /// A new message was stored (incoming, or a local outgoing one).
    Message(Box<MessageRecord>),
    /// Delivery status changed for an outgoing message.
    MessageStatus {
        packet_id: u32,
        outgoing: bool,
        status: MessageStatus,
        error: Option<String>,
    },
    /// A position report.
    Position {
        node_num: u32,
        position: Box<Position>,
    },
    /// A telemetry sample.
    Telemetry {
        node_num: u32,
        telemetry: Box<Telemetry>,
    },
    /// A traceroute response.
    Traceroute {
        packet_id: u32,
        from: u32,
        route: Vec<u32>,
        snr_towards: Vec<i32>,
        route_back: Vec<u32>,
        snr_back: Vec<i32>,
    },
    /// Raw queue status from the device.
    QueueStatus(Box<QueueStatus>),
    /// A line of device debug output.
    DeviceLog(String),
    /// A BLE device needs the passkey shown on its screen to finish pairing.
    BlePairingRequest { address: String },
    /// The device rebooted.
    Rebooted { from_dfu: bool },
    /// Non-fatal error worth surfacing to the user.
    Error(String),
}

/// Commands the UI dispatches to the core.
#[derive(Debug)]
pub enum CoreCommand {
    /// Connect to a device (replacing any existing connection).
    Connect(DeviceAddress),
    /// Gracefully disconnect and stay idle.
    Disconnect,
    /// Send a text message.
    SendText {
        text: String,
        channel: u32,
        /// `None` or `Some(BROADCAST_ADDR)` sends to the channel timeline.
        to: Option<u32>,
        /// Set for emoji reactions.
        reply_id: Option<u32>,
    },
    /// Ask a node for a position report.
    RequestPosition(u32),
    /// Ask for a traceroute to a node.
    Traceroute(u32),
    /// Ask a (usually remote) node to resend its node database.
    RequestNodeList(u32),
    /// Star / unstar a node.
    SetFavorite { node_num: u32, favorite: bool },
    /// Ignore / unignore a node.
    SetIgnored { node_num: u32, ignored: bool },
    /// Remove a node from the local database.
    RemoveNode(u32),
    /// Store a shared contact on the device, importing its public key.
    AddContact(Box<SharedContact>),
    /// Change the connected node's owner name.
    SetOwner {
        long_name: String,
        short_name: String,
        is_licensed: bool,
    },
    /// Replace a channel slot.
    SetChannel(Box<Channel>),
    /// Apply a device config section.
    SetConfig(Box<Config>),
    /// Apply a module config section.
    SetModuleConfig(Box<ModuleConfig>),
    /// Re-run the config handshake with the current device.
    Resync,
    /// Change the automatic-reconnect preference at runtime.
    SetAutoReconnect(bool),
    /// Supply the BLE pairing passkey shown on the device's screen.
    SubmitBlePasskey(u32),
    /// Reboot a node (0 seconds = immediately).
    Reboot { dest: u32, seconds: i32 },
    /// Shut a node down.
    Shutdown { dest: u32, seconds: i32 },
    /// Factory reset a node (`full_device` also clears the node db).
    FactoryReset { dest: u32, full_device: bool },
    /// Ask the device for its logs (serial/BLE only).
    ShutdownCore,
}
