//! In-memory mirror of the connected device's state.
//!
//! Everything received during the handshake and from live packets is folded
//! in here, so the UI can be served without re-reading the database and
//! outbound logic can consult the latest node/channel data.

use std::collections::HashMap;

use meshtastic_protobufs::meshtastic::{
    Channel, Config, DeviceMetadata, ModuleConfig, MyNodeInfo, NodeInfo,
};
use mt_persistence::MessageStatus;

/// An outbound message being tracked until it is acknowledged.
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub packet_id: u32,
    /// Destination node number; used to tell a destination ACK (target
    /// received the packet) from a mesh-level implicit ACK.
    pub to: u32,
    /// Whether a routing acknowledgement is expected.
    pub want_ack: bool,
    /// Unix seconds the message was handed to the radio.
    pub sent_at: i64,
    pub status: MessageStatus,
}

/// Mutable state owned by the supervisor for one connection.
#[derive(Debug, Default)]
pub struct MeshState {
    pub my_node_num: Option<u32>,
    pub my_info: Option<MyNodeInfo>,
    pub metadata: Option<DeviceMetadata>,
    pub nodes: HashMap<u32, NodeInfo>,
    /// Latest observed RSSI per node (not part of `NodeInfo`).
    pub rssi: HashMap<u32, i32>,
    pub channels: Vec<Channel>,
    pub device_configs: Vec<Config>,
    pub module_configs: Vec<ModuleConfig>,
    /// Messages awaiting acknowledgement / status, keyed by packet id.
    pub outbound: HashMap<u32, OutboundMessage>,
    /// Nonce of the handshake that completed, for diagnostics.
    pub handshake_nonce: Option<u32>,
}

impl MeshState {
    pub fn my_num(&self) -> Option<u32> {
        self.my_node_num
    }

    /// Insert or merge a node, returning the stored value.
    pub fn upsert_node(&mut self, mut info: NodeInfo) -> NodeInfo {
        if let Some(existing) = self.nodes.get(&info.num) {
            // Keep previously learned details when a live update omits them.
            if info.user.is_none() {
                info.user = existing.user.clone();
            }
            if info.position.is_none() {
                info.position = existing.position;
            }
            if info.device_metrics.is_none() {
                info.device_metrics = existing.device_metrics;
            }
            if info.hops_away.is_none() {
                info.hops_away = existing.hops_away;
            }
        }
        self.nodes.insert(info.num, info.clone());
        info
    }

    /// Record that a node was heard, updating signal info when present.
    pub fn note_heard(
        &mut self,
        num: u32,
        snr: f32,
        rssi: i32,
        hops_away: Option<u32>,
        last_heard: u32,
    ) {
        if rssi != 0 {
            self.rssi.insert(num, rssi);
        }
        if let Some(node) = self.nodes.get_mut(&num) {
            if snr != 0.0 {
                node.snr = snr;
            }
            if last_heard != 0 {
                node.last_heard = last_heard;
            }
            if hops_away.is_some() {
                node.hops_away = hops_away;
            }
        }
    }

    /// Apply a device config section, replacing any previous one.
    pub fn apply_config(&mut self, config: Config) {
        self.device_configs.retain(|c| {
            !matches!(
                (c.payload_variant.as_ref(), config.payload_variant.as_ref()),
                (Some(a), Some(b)) if std::mem::discriminant(a) == std::mem::discriminant(b)
            )
        });
        self.device_configs.push(config);
    }

    /// Apply a module config section, replacing any previous one.
    pub fn apply_module_config(&mut self, config: ModuleConfig) {
        self.module_configs.retain(|c| {
            !matches!(
                (c.payload_variant.as_ref(), config.payload_variant.as_ref()),
                (Some(a), Some(b)) if std::mem::discriminant(a) == std::mem::discriminant(b)
            )
        });
        self.module_configs.push(config);
    }

    /// Apply a channel slot by index.
    pub fn apply_channel(&mut self, channel: Channel) {
        match self.channels.iter_mut().find(|c| c.index == channel.index) {
            Some(slot) => *slot = channel,
            None => {
                self.channels.push(channel);
                self.channels.sort_by_key(|c| c.index);
            }
        }
    }

    /// Add or replace an outbound tracking entry.
    pub fn track_outbound(&mut self, msg: OutboundMessage) {
        self.outbound.insert(msg.packet_id, msg);
    }

    /// Set an outbound message's status, returning true when it changed.
    pub fn set_outbound_status(&mut self, packet_id: u32, status: MessageStatus) -> bool {
        match self.outbound.get_mut(&packet_id) {
            Some(msg) if msg.status != status => {
                msg.status = status;
                true
            }
            _ => false,
        }
    }
}
