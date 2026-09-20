//! Ingest of `FromRadio` messages: handshake data, live packets and
//! administrative responses. Everything is folded into [`MeshState`],
//! persisted when a device database is open, and announced to the UI.

use meshtastic_protobufs::meshtastic::{
    AdminMessage, Channel, Config, Data, DeviceMetadata, FromRadio, MeshPacket, ModuleConfig,
    MyNodeInfo, NodeInfo, PortNum, Position, RouteDiscovery, Telemetry, User, admin_message,
    from_radio, mesh_packet,
};
use mt_persistence::{MessageStatus, now_unix};
use prost::Message as _;

use crate::events::CoreEvent;
use crate::supervisor::Supervisor;

impl Supervisor {
    /// Route one `FromRadio` message.
    pub(crate) async fn handle_from_radio(&mut self, msg: FromRadio) {
        use from_radio::PayloadVariant as V;
        match msg.payload_variant {
            Some(V::MyInfo(info)) => self.on_my_info(info),
            Some(V::Metadata(metadata)) => self.on_metadata(metadata),
            Some(V::Config(config)) => self.on_config(config),
            Some(V::ModuleConfig(config)) => self.on_module_config(config),
            Some(V::Channel(channel)) => self.on_channel(channel),
            Some(V::NodeInfo(node)) => self.on_node_info(node),
            Some(V::Packet(packet)) => self.handle_packet(packet),
            Some(V::QueueStatus(status)) => self.handle_queue_status(status),
            Some(V::ConfigCompleteId(nonce)) => self.on_config_complete(nonce).await,
            Some(V::Rebooted(from_dfu)) => self.emit(CoreEvent::Rebooted { from_dfu }),
            Some(V::LogRecord(record)) => {
                let line = if record.source.is_empty() {
                    record.message
                } else {
                    format!("{}: {}", record.source, record.message)
                };
                self.emit(CoreEvent::DeviceLog(line));
            }
            // Not yet surfaced in the UI.
            Some(V::ClientNotification(_))
            | Some(V::XmodemPacket(_))
            | Some(V::MqttClientProxyMessage(_))
            | Some(V::FileInfo(_))
            | Some(V::DeviceuiConfig(_))
            | None => {}
        }
    }

    fn on_my_info(&mut self, info: MyNodeInfo) {
        self.state.my_node_num = Some(info.my_node_num);
        self.state.my_info = Some(info.clone());
        self.emit(CoreEvent::MyInfo(Box::new(info.clone())));

        // Node number unlocks the per-device database.
        if self.db_node != Some(info.my_node_num) {
            self.open_device_db(info.my_node_num);
            if let Some(db) = &self.db {
                let _ = db.set_my_node_num(info.my_node_num);
            }
        }
    }

    fn on_metadata(&mut self, metadata: DeviceMetadata) {
        self.state.metadata = Some(metadata.clone());
        if let Some(db) = &self.db {
            if !metadata.firmware_version.is_empty() {
                let _ = db.set_firmware_version(&metadata.firmware_version);
            }
        }
        self.emit(CoreEvent::Metadata(Box::new(metadata)));
    }

    fn on_config(&mut self, config: Config) {
        self.state.apply_config(config.clone());
        if let Some(db) = &self.db {
            let _ = db.replace_configs(std::slice::from_ref(&config));
        }
        self.emit(CoreEvent::Config(Box::new(config)));
    }

    fn on_module_config(&mut self, config: ModuleConfig) {
        self.state.apply_module_config(config.clone());
        if let Some(db) = &self.db {
            let _ = db.replace_module_configs(std::slice::from_ref(&config));
        }
        self.emit(CoreEvent::ModuleConfig(Box::new(config)));
    }

    fn on_channel(&mut self, channel: Channel) {
        self.state.apply_channel(channel.clone());
        if let Some(db) = &self.db {
            let _ = db.upsert_channel(&channel);
        }
        self.emit(CoreEvent::Channel(Box::new(channel)));
    }

    fn on_node_info(&mut self, node: NodeInfo) {
        let stored = self.state.upsert_node(node);
        if let Some(db) = &self.db {
            let _ = db.upsert_node(&stored);
        }
        self.emit(CoreEvent::Node(Box::new(stored)));
    }

    /// Handle a mesh packet by port number.
    pub(crate) fn handle_packet(&mut self, packet: MeshPacket) {
        let data = match &packet.payload_variant {
            Some(mesh_packet::PayloadVariant::Decoded(data)) => data.clone(),
            // Encrypted packets carry no readable payload (yet); still note
            // that the sender was heard.
            _ => {
                self.touch_node(&packet);
                return;
            }
        };

        match PortNum::try_from(data.portnum).unwrap_or(PortNum::UnknownApp) {
            PortNum::TextMessageApp | PortNum::AlertApp => self.on_text_packet(&packet, &data),
            PortNum::RoutingApp => self.handle_routing(packet.from, &data),
            PortNum::PositionApp => self.on_position_packet(&packet, &data),
            PortNum::TelemetryApp => self.on_telemetry_packet(&packet, &data),
            PortNum::NodeinfoApp => self.on_nodeinfo_packet(&packet, &data),
            PortNum::TracerouteApp => self.on_traceroute_packet(&packet, &data),
            PortNum::AdminApp => self.on_admin_packet(&packet, &data),
            _ => {}
        }

        // Every inbound packet proves its sender is alive, including
        // routing acks and traceroute/admin replies that carry no other
        // node bookkeeping. Applied after the port handlers so a payload's
        // stale timestamp cannot overwrite the arrival time.
        self.touch_node(&packet);
    }

    fn on_text_packet(&mut self, packet: &MeshPacket, _data: &Data) {
        let outgoing = self
            .state
            .my_num()
            .map(|my| packet.from == my)
            .unwrap_or(false);
        let status = if outgoing {
            MessageStatus::Enroute
        } else {
            MessageStatus::Delivered
        };

        if let Some(db) = &self.db {
            let sent_at = (packet.rx_time != 0).then_some(packet.rx_time as i64);
            let _ = db.insert_message(packet, outgoing, status, sent_at);
        }
        self.emit_stored_message(packet.id, outgoing);

        if outgoing {
            self.update_status(packet.id, true, MessageStatus::Enroute, None);
        }
    }

    fn on_position_packet(&mut self, packet: &MeshPacket, data: &Data) {
        let Ok(position) = Position::decode(data.payload.as_slice()) else {
            return;
        };
        let node_num = packet.from;

        // Signal metrics and last-heard are applied by the caller's final
        // `touch_node`, so this only folds in the position itself.
        if let Some(mut node) = self.state.nodes.get(&node_num).cloned() {
            node.position = Some(position.clone());
            let node = self.state.upsert_node(node);
            if let Some(db) = &self.db {
                let _ = db.upsert_node(&node);
                let _ = db.insert_position(node_num, &position);
            }
        } else if let Some(db) = &self.db {
            let _ = db.insert_position(node_num, &position);
        }

        self.emit(CoreEvent::Position {
            node_num,
            position: Box::new(position),
        });
    }

    fn on_telemetry_packet(&mut self, packet: &MeshPacket, data: &Data) {
        let Ok(telemetry) = Telemetry::decode(data.payload.as_slice()) else {
            return;
        };
        if let Some(db) = &self.db {
            let _ = db.insert_telemetry(packet.from, &telemetry);
        }
        self.emit(CoreEvent::Telemetry {
            node_num: packet.from,
            telemetry: Box::new(telemetry),
        });
    }

    fn on_nodeinfo_packet(&mut self, packet: &MeshPacket, data: &Data) {
        // Node info can arrive either as a NodeInfo payload or wrapped in a
        // User payload; try NodeInfo first, then fall back to User.
        if let Ok(node) = NodeInfo::decode(data.payload.as_slice()) {
            self.on_node_info(node);
        } else if let Ok(user) = User::decode(data.payload.as_slice()) {
            let mut node = NodeInfo {
                num: packet.from,
                user: Some(user),
                last_heard: packet.rx_time,
                snr: packet.rx_snr,
                ..Default::default()
            };
            if let Some(existing) = self.state.nodes.get(&packet.from) {
                node.position = existing.position.clone();
                node.device_metrics = existing.device_metrics.clone();
            }
            self.on_node_info(node);
        }
    }

    /// Handle a traceroute packet.
    ///
    /// The wire payload stores intermediate hops only; the endpoints are
    /// rebuilt here so consumers get the full path. For both directions,
    /// SNR list index `i` labels the link between hop `i` and hop `i + 1`.
    fn on_traceroute_packet(&mut self, packet: &MeshPacket, data: &Data) {
        // Requests are promiscuously relayed; only responses carry a route.
        if data.want_response {
            return;
        }
        let Ok(discovery) = RouteDiscovery::decode(data.payload.as_slice()) else {
            return;
        };

        // Firmware leaves `dest`/`source` unset on responses; the packet
        // header says it all: `to` is the trace origin, `from` the target.
        let origin = if data.dest != 0 {
            data.dest
        } else if packet.to != 0 && packet.to != mt_protocol::constants::BROADCAST_ADDR {
            packet.to
        } else {
            self.state.my_num().unwrap_or(0)
        };
        let target = if data.source != 0 {
            data.source
        } else {
            packet.from
        };

        let mut route = Vec::with_capacity(discovery.route.len() + 2);
        route.push(origin);
        route.extend(discovery.route.iter().copied());
        route.push(target);

        let mut route_back = Vec::with_capacity(discovery.route_back.len() + 2);
        route_back.push(target);
        route_back.extend(discovery.route_back.iter().copied());
        route_back.push(origin);

        tracing::debug!(
            packet_id = data.request_id,
            target,
            ?route,
            ?route_back,
            snr_towards = ?discovery.snr_towards,
            snr_back = ?discovery.snr_back,
            "traceroute response"
        );
        self.emit(CoreEvent::Traceroute {
            packet_id: data.request_id,
            target,
            route,
            snr_towards: discovery.snr_towards,
            route_back,
            snr_back: discovery.snr_back,
        });
    }

    fn on_admin_packet(&mut self, packet: &MeshPacket, data: &Data) {
        let Ok(admin) = AdminMessage::decode(data.payload.as_slice()) else {
            return;
        };
        // Firmware echoes a session passkey in admin responses; keep the
        // latest so our own writes carry it (required by newer firmware).
        if !admin.session_passkey.is_empty() {
            self.session_passkey = admin.session_passkey.clone();
        }
        match admin.payload_variant {
            Some(admin_message::PayloadVariant::GetOwnerResponse(user)) => {
                let node_num = self.state.my_num().unwrap_or(packet.from);
                if let Some(mut node) = self.state.nodes.get(&node_num).cloned() {
                    node.user = Some(user);
                    let node = self.state.upsert_node(node);
                    if let Some(db) = &self.db {
                        let _ = db.upsert_node(&node);
                    }
                    self.emit(CoreEvent::Node(Box::new(node)));
                }
            }
            Some(admin_message::PayloadVariant::GetDeviceMetadataResponse(metadata)) => {
                self.on_metadata(metadata);
            }
            Some(admin_message::PayloadVariant::GetChannelResponse(channel)) => {
                self.on_channel(channel);
            }
            Some(admin_message::PayloadVariant::GetConfigResponse(config)) => {
                self.on_config(config);
            }
            Some(admin_message::PayloadVariant::GetModuleConfigResponse(config)) => {
                self.on_module_config(config);
            }
            _ => {}
        }
    }

    /// Update a known node's "heard" data (signal, hop count, timestamp).
    pub(crate) fn touch_node(&mut self, packet: &MeshPacket) {
        let num = packet.from;
        if num == 0 || !self.state.nodes.contains_key(&num) {
            return;
        }
        let hops_away =
            (packet.hop_start > 0).then(|| packet.hop_start.saturating_sub(packet.hop_limit));
        self.state.note_heard(
            num,
            packet.rx_snr,
            packet.rx_rssi as i32,
            hops_away,
            packet_last_heard(packet),
        );
        if packet.rx_rssi != 0 {
            self.emit(CoreEvent::Rssi {
                node_num: num,
                rssi: packet.rx_rssi as i32,
            });
        }
        if let Some(node) = self.state.nodes.get(&num).cloned() {
            if let Some(db) = &self.db {
                let _ = db.upsert_node(&node);
            }
            self.emit(CoreEvent::Node(Box::new(node)));
        }
    }
}

/// Timestamp to record for an arriving packet.
///
/// The radio stamps `rx_time` when it has a time source (GPS or a phone-set
/// clock); when it does not, it sends zero and the host clock is the only
/// reference. Timestamps from the radio's unsynced clock can also run ahead
/// of the host, so they are clamped to now. Mirrors the official clients,
/// which normalize every packet to `packet.rx_time` or their own clock.
fn packet_last_heard(packet: &MeshPacket) -> u32 {
    let now = now_unix().clamp(0, u32::MAX as i64) as u32;
    match packet.rx_time {
        0 => now,
        t if t > now => now,
        t => t,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshtastic_protobufs::meshtastic::{Routing, routing};
    use tokio::sync::{broadcast, mpsc};

    use crate::supervisor::CoreConfig;

    fn supervisor() -> Supervisor {
        let (event_tx, _) = broadcast::channel(16);
        let (_cmd_tx, cmd_rx) = mpsc::channel(4);
        let mut supervisor = Supervisor::new(CoreConfig::default(), event_tx, cmd_rx);
        supervisor.state.my_node_num = Some(1);
        supervisor
    }

    fn routing_ack(from: u32, rx_time: u32) -> MeshPacket {
        MeshPacket {
            from,
            to: 1,
            rx_time,
            payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
                portnum: PortNum::RoutingApp as i32,
                request_id: 42,
                payload: Routing {
                    variant: Some(routing::Variant::ErrorReason(routing::Error::None as i32)),
                }
                .encode_to_vec(),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    #[test]
    fn routing_ack_refreshes_sender_last_heard() {
        let mut supervisor = supervisor();
        supervisor.state.upsert_node(NodeInfo {
            num: 2,
            ..Default::default()
        });

        supervisor.handle_packet(routing_ack(2, 1_700_000_000));

        assert_eq!(supervisor.state.nodes[&2].last_heard, 1_700_000_000);
    }

    #[test]
    fn unstamped_packet_falls_back_to_host_clock_and_clamps_future() {
        let mut supervisor = supervisor();
        supervisor.state.upsert_node(NodeInfo {
            num: 2,
            ..Default::default()
        });

        supervisor.handle_packet(routing_ack(2, 0));
        let heard = supervisor.state.nodes[&2].last_heard;
        assert!(heard > 0, "zero rx_time must still refresh last_heard");

        let now = now_unix() as u32;
        supervisor.handle_packet(routing_ack(2, now + 86_400));
        assert!(
            supervisor.state.nodes[&2].last_heard <= now_unix() as u32,
            "future timestamps must be clamped to the host clock"
        );
    }

    #[test]
    fn outgoing_packet_refreshes_local_node() {
        let mut supervisor = supervisor();
        supervisor.state.upsert_node(NodeInfo {
            num: 1,
            ..Default::default()
        });

        supervisor.handle_packet(routing_ack(1, 1_700_000_000));

        assert_eq!(supervisor.state.nodes[&1].last_heard, 1_700_000_000);
    }
}
