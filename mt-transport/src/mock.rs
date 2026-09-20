//! A simulated Meshtastic node running in-process.
//!
//! It speaks the exact same event protocol as the real transports: it
//! answers the two-stage handshake with a complete config download
//! (myInfo, metadata, device + module config, channels, a small node db)
//! and then simulates mesh activity:
//!
//! - `QueueStatus` + routing acknowledgements for messages the client
//!   sends,
//! - replies from a cast of fake nodes,
//! - periodic telemetry and position reports.
//!
//! This makes the whole application testable and demo-able without any
//! radio hardware.

use std::collections::HashSet;
use std::time::Duration;

use meshtastic_protobufs::meshtastic::{
    AdminMessage, Channel, ChannelSettings, Config, Data, DeviceMetrics, EnvironmentMetrics,
    FromRadio, MeshPacket, ModuleConfig, MyNodeInfo, NodeInfo, Position, RouteDiscovery, Routing,
    Telemetry, ToRadio, User, admin_message, channel, config, from_radio, mesh_packet,
    module_config, routing, telemetry,
};
use prost::Message;
use rand::RngExt as _;
use tokio::sync::mpsc;

use mt_protocol::constants::{BROADCAST_ADDR, HANDSHAKE_NONCE_1, HANDSHAKE_NONCE_2};

use crate::{TransportCommand, TransportError, TransportEvent};

/// The local (simulated) node number. Deliberately memorable.
const MY_NODE_NUM: u32 = 0x0042_A1B2;
const FIRMWARE_VERSION: &str = "2.7.8.b8942c9";

struct FakeNode {
    num: u32,
    user: User,
    position: Position,
}

fn cast() -> Vec<FakeNode> {
    let mk = |num: u32, id: &str, long: &str, short: &str, lat: i32, lon: i32, alt: i32| FakeNode {
        num,
        user: User {
            id: id.to_string(),
            long_name: long.to_string(),
            short_name: short.to_string(),
            hw_model: 43, // HELTEC_V3
            ..Default::default()
        },
        position: Position {
            latitude_i: Some(lat),
            longitude_i: Some(lon),
            altitude: Some(alt),
            time: 0,
            ..Default::default()
        },
    };
    vec![
        mk(
            0x7A11_0001,
            "!7a110001",
            "Base Camp",
            "Base",
            40_017_000,
            -105_283_000,
            1_600,
        ),
        mk(
            0x7A11_0002,
            "!7a110002",
            "Ridge Repeater",
            "Rdge",
            40_022_500,
            -105_295_000,
            2_650,
        ),
        mk(
            0x7A11_0003,
            "!7a110003",
            "Trail Buddy",
            "Trail",
            40_012_400,
            -105_271_800,
            1_730,
        ),
    ]
}

fn my_user() -> User {
    User {
        id: format!("!{:08x}", MY_NODE_NUM),
        long_name: "Mock Station".to_string(),
        short_name: "MckS".to_string(),
        hw_model: 43,
        ..Default::default()
    }
}

fn primary_channel() -> Channel {
    Channel {
        index: 0,
        role: channel::Role::Primary as i32,
        settings: Some(ChannelSettings {
            name: "Default".to_string(),
            psk: vec![1; 32], // standard "default" key
            ..Default::default()
        }),
    }
}

fn config_download() -> Vec<FromRadio> {
    let mut msgs = Vec::new();

    msgs.push(FromRadio {
        id: 0,
        payload_variant: Some(from_radio::PayloadVariant::MyInfo(MyNodeInfo {
            my_node_num: MY_NODE_NUM,
            reboot_count: 12,
            min_app_version: 30_200,
            ..Default::default()
        })),
    });

    msgs.push(FromRadio {
        id: 0,
        payload_variant: Some(from_radio::PayloadVariant::Metadata(
            meshtastic_protobufs::meshtastic::DeviceMetadata {
                firmware_version: FIRMWARE_VERSION.to_string(),
                device_state_version: 23,
                can_shutdown: true,
                ..Default::default()
            },
        )),
    });

    // Device + LoRa + position config sections.
    msgs.push(FromRadio {
        id: 0,
        payload_variant: Some(from_radio::PayloadVariant::Config(Config {
            payload_variant: Some(config::PayloadVariant::Device(config::DeviceConfig {
                role: config::device_config::Role::Client as i32,
                node_info_broadcast_secs: 900,
                ..Default::default()
            })),
        })),
    });
    msgs.push(FromRadio {
        id: 0,
        payload_variant: Some(from_radio::PayloadVariant::Config(Config {
            payload_variant: Some(config::PayloadVariant::Lora(config::LoRaConfig {
                region: config::lo_ra_config::RegionCode::Us as i32,
                modem_preset: config::lo_ra_config::ModemPreset::LongFast as i32,
                hop_limit: 3,
                tx_enabled: true,
                tx_power: 27,
                use_preset: true,
                channel_num: 38,
                ..Default::default()
            })),
        })),
    });
    msgs.push(FromRadio {
        id: 0,
        payload_variant: Some(from_radio::PayloadVariant::Config(Config {
            payload_variant: Some(config::PayloadVariant::Position(config::PositionConfig {
                position_broadcast_secs: 900,
                ..Default::default()
            })),
        })),
    });

    // A couple of module configs.
    msgs.push(FromRadio {
        id: 0,
        payload_variant: Some(from_radio::PayloadVariant::ModuleConfig(ModuleConfig {
            payload_variant: Some(module_config::PayloadVariant::Telemetry(
                module_config::TelemetryConfig {
                    device_update_interval: 300,
                    environment_update_interval: 120,
                    ..Default::default()
                },
            )),
        })),
    });

    msgs.push(FromRadio {
        id: 0,
        payload_variant: Some(from_radio::PayloadVariant::Channel(primary_channel())),
    });

    // Our own entry in the NodeDB.
    let mut own = NodeInfo {
        num: MY_NODE_NUM,
        user: Some(my_user()),
        position: Some(Position {
            latitude_i: Some(40_015_100),
            longitude_i: Some(-105_280_500),
            altitude: Some(1_655),
            ..Default::default()
        }),
        snr: 0.0,
        ..Default::default()
    };
    own.position.as_mut().unwrap().time = now_unix() - 30;
    msgs.push(FromRadio {
        id: 0,
        payload_variant: Some(from_radio::PayloadVariant::NodeInfo(own)),
    });

    msgs
}

/// The other nodes in the mock NodeDB, streamed only in Stage 2.
fn other_node_infos(removed: &HashSet<u32>) -> Vec<FromRadio> {
    cast()
        .into_iter()
        .filter(|node| !removed.contains(&node.num))
        .map(|node| {
            let mut ni = NodeInfo {
                num: node.num,
                user: Some(node.user),
                position: Some(node.position),
                snr: -12.5,
                ..Default::default()
            };
            ni.position.as_mut().unwrap().time = now_unix() - rand::rng().random_range(60..3_600);
            FromRadio {
                id: 0,
                payload_variant: Some(from_radio::PayloadVariant::NodeInfo(ni)),
            }
        })
        .collect()
}

/// The Stage 2 download: MyInfo, our own node and the rest of the NodeDB.
fn node_download(removed: &HashSet<u32>) -> Vec<FromRadio> {
    let mut msgs: Vec<FromRadio> = config_download()
        .into_iter()
        .filter(|msg| {
            matches!(
                msg.payload_variant,
                Some(from_radio::PayloadVariant::MyInfo(_))
                    | Some(from_radio::PayloadVariant::NodeInfo(_))
            )
        })
        .collect();
    msgs.extend(other_node_infos(removed));
    msgs
}

fn now_unix() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0)
}

/// Wrap a MeshPacket into a FromRadio for delivery to the client.
fn packet_to_from_radio(p: MeshPacket) -> FromRadio {
    FromRadio {
        id: 0,
        payload_variant: Some(from_radio::PayloadVariant::Packet(p)),
    }
}

fn text_packet(from: u32, to: u32, channel: u32, text: &str) -> MeshPacket {
    MeshPacket {
        from,
        to,
        channel,
        id: rand::rng().random_range(1..u32::MAX),
        rx_time: now_unix(),
        rx_snr: -8.0,
        hop_limit: 3,
        priority: 10,
        payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
            portnum: meshtastic_protobufs::meshtastic::PortNum::TextMessageApp as i32,
            payload: text.as_bytes().to_vec(),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// A routing report ("ACK") for the packet `request_id`.
fn routing_ack(from: u32, request_id: u32, channel: u32) -> MeshPacket {
    MeshPacket {
        from,
        to: 0, // to the phone/client
        channel,
        rx_time: now_unix(),
        rx_snr: -9.0,
        hop_limit: 3,
        priority: 10,
        payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
            portnum: meshtastic_protobufs::meshtastic::PortNum::RoutingApp as i32,
            request_id,
            payload: encode(&Routing {
                variant: Some(routing::Variant::ErrorReason(routing::Error::None as i32)),
            }),
            ..Default::default()
        })),
        ..Default::default()
    }
}

pub(crate) async fn run(
    name: &str,
    mut cmd_rx: mpsc::Receiver<TransportCommand>,
    evt_tx: mpsc::Sender<TransportEvent>,
) -> Result<(), TransportError> {
    tracing::info!(name, "mock radio starting");
    let _ = evt_tx.send(TransportEvent::Connected).await;

    // Timers for simulated mesh activity.
    let mut chatter = tokio::time::interval(Duration::from_secs(45));
    chatter.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut telemetry_tick = tokio::time::interval(Duration::from_secs(60));
    telemetry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // First tick of an interval fires immediately: consume it so chatter
    // only starts after the client has had time to finish the handshake.
    chatter.tick().await;
    telemetry_tick.tick().await;

    let send = |msg: FromRadio, evt_tx: &mpsc::Sender<TransportEvent>| {
        let _ = evt_tx.try_send(TransportEvent::FromRadio(msg));
    };

    // Nodes the client has asked the device to forget.
    let mut removed: HashSet<u32> = HashSet::new();

    loop {
        tokio::select! {
            biased;

            maybe_cmd = cmd_rx.recv() => {
                match maybe_cmd {
                    None | Some(TransportCommand::Disconnect) => {
                        tracing::info!("mock radio shutting down");
                        return Ok(());
                    }
                    Some(TransportCommand::Send(msg)) => handle_toradio(msg, &evt_tx, &mut removed).await,
                    Some(TransportCommand::BlePasskey(_)) => {}
                }
            }

            _ = chatter.tick() => {
                let cast = cast();
                let node = &cast[rand::rng().random_range(0..cast.len())];
                let lines = [
                    "copy that, 5 by 9",
                    "anyone copy?",
                    "sun's setting up here, beautiful",
                    "moving to the next ridge",
                    "battery at 68%",
                ];
                let text = lines[rand::rng().random_range(0..lines.len())];
                send(
                    packet_to_from_radio(text_packet(node.num, BROADCAST_ADDR, 0, text)),
                    &evt_tx,
                );
            }

            _ = telemetry_tick.tick() => {
                let cast = cast();
                let node = &cast[0];
                send(
                    packet_to_from_radio(MeshPacket {
                        from: node.num,
                        to: BROADCAST_ADDR,
                        channel: 0,
                        id: rand::rng().random_range(1..u32::MAX),
                        rx_time: now_unix(),
                        hop_limit: 3,
                        payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
                            portnum: meshtastic_protobufs::meshtastic::PortNum::TelemetryApp as i32,
                            payload: encode(&Telemetry {
                                time: now_unix(),
                                variant: Some(telemetry::Variant::EnvironmentMetrics(
                                    EnvironmentMetrics {
                                        temperature: Some(18.4),
                                        relative_humidity: Some(41.0),
                                        barometric_pressure: Some(835.0),
                                        ..Default::default()
                                    },
                                )),
                            }),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                    &evt_tx,
                );
                send(
                    packet_to_from_radio(MeshPacket {
                        from: MY_NODE_NUM,
                        to: BROADCAST_ADDR,
                        channel: 0,
                        id: rand::rng().random_range(1..u32::MAX),
                        rx_time: now_unix(),
                        hop_limit: 3,
                        payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
                            portnum: meshtastic_protobufs::meshtastic::PortNum::TelemetryApp as i32,
                            payload: encode(&Telemetry {
                                time: now_unix(),
                                variant: Some(telemetry::Variant::DeviceMetrics(DeviceMetrics {
                                    battery_level: Some(87),
                                    voltage: Some(4.12),
                                    channel_utilization: Some(12.5),
                                    air_util_tx: Some(2.1),
                                    ..Default::default()
                                })),
                            }),
                            ..Default::default()
                        })),
                        ..Default::default()
                    }),
                    &evt_tx,
                );
            }
        }
    }
}

async fn handle_toradio(
    msg: ToRadio,
    evt_tx: &mpsc::Sender<TransportEvent>,
    removed: &mut HashSet<u32>,
) {
    use meshtastic_protobufs::meshtastic::to_radio::PayloadVariant as To;
    match msg.payload_variant {
        Some(To::WantConfigId(nonce)) => {
            // Two-stage handshake, mirroring the firmware: Stage 1 streams the
            // config and channels, Stage 2 streams the node database.
            let mut msgs = if nonce == HANDSHAKE_NONCE_2 {
                node_download(removed)
            } else if nonce == HANDSHAKE_NONCE_1 {
                config_download()
            } else {
                let mut legacy = config_download();
                legacy.extend(other_node_infos(removed));
                legacy
            };
            msgs.push(FromRadio {
                id: 0,
                payload_variant: Some(from_radio::PayloadVariant::ConfigCompleteId(nonce)),
            });
            for m in msgs {
                let _ = evt_tx.send(TransportEvent::FromRadio(m)).await;
            }
        }
        Some(To::Packet(p)) => handle_mesh_packet(p, evt_tx, removed).await,
        Some(To::Disconnect(_)) => {}
        Some(To::Heartbeat(_)) => {
            // Real nodes stay silent; nothing to do.
        }
        _ => {}
    }
}

async fn handle_mesh_packet(
    p: MeshPacket,
    evt_tx: &mpsc::Sender<TransportEvent>,
    removed: &mut HashSet<u32>,
) {
    if let Some(mesh_packet::PayloadVariant::Decoded(data)) = &p.payload_variant {
        use meshtastic_protobufs::meshtastic::PortNum;
        let port = PortNum::try_from(data.portnum).unwrap_or(PortNum::UnknownApp);

        // Queue acceptance for any packet we "transmit".
        let _ = evt_tx
            .send(TransportEvent::FromRadio(FromRadio {
                id: 0,
                payload_variant: Some(from_radio::PayloadVariant::QueueStatus(
                    meshtastic_protobufs::meshtastic::QueueStatus {
                        res: 0,
                        free: 100,
                        maxlen: 100,
                        mesh_packet_id: p.id,
                    },
                )),
            }))
            .await;

        match port {
            PortNum::TextMessageApp => {
                // Real firmware sends two kinds of "no error" routing report
                // for a direct message: an implicit ACK from the local node
                // when a neighbour rebroadcasts the packet (so the message
                // reached the mesh, but not necessarily the destination),
                // then the destination's own ACK. Channel broadcasts only
                // ever get the local implicit ACK.
                let ack_id = p.id;
                let is_direct = p.to != BROADCAST_ADDR;
                let destination = p.to;
                let channel = p.channel;
                let evt_tx = evt_tx.clone();
                tokio::spawn(async move {
                    if is_direct {
                        tokio::time::sleep(Duration::from_millis(700)).await;
                        let _ = evt_tx
                            .send(TransportEvent::FromRadio(packet_to_from_radio(
                                routing_ack(MY_NODE_NUM, ack_id, channel),
                            )))
                            .await;
                    }

                    let ack_from = if is_direct { destination } else { MY_NODE_NUM };
                    tokio::time::sleep(Duration::from_millis(if is_direct { 800 } else { 1_500 }))
                        .await;
                    let _ = evt_tx
                        .send(TransportEvent::FromRadio(packet_to_from_radio(
                            routing_ack(ack_from, ack_id, channel),
                        )))
                        .await;

                    // A remote node replies after a while.
                    let cast = cast();
                    let node = &cast[rand::rng().random_range(0..cast.len())];
                    tokio::time::sleep(Duration::from_millis(2_000)).await;
                    let _ = evt_tx
                        .send(TransportEvent::FromRadio(packet_to_from_radio(
                            text_packet(node.num, BROADCAST_ADDR, channel, "roger that 👍"),
                        )))
                        .await;
                });
            }
            PortNum::TracerouteApp => {
                // Answer a request with a response carrying one intermediate
                // hop, the way real firmware fills the RouteDiscovery: the
                // endpoints themselves are not in the payload.
                if data.want_response {
                    let request_id = p.id;
                    let target = p.to;
                    let channel = p.channel;
                    let reply_id = rand::rng().random_range(1..u32::MAX);
                    let evt_tx = evt_tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        let relay = 0x0BAD_CAFE;
                        let _ = evt_tx
                            .send(TransportEvent::FromRadio(packet_to_from_radio(
                                MeshPacket {
                                    from: target,
                                    to: MY_NODE_NUM,
                                    channel,
                                    id: reply_id,
                                    rx_time: now_unix(),
                                    payload_variant: Some(mesh_packet::PayloadVariant::Decoded(
                                        Data {
                                            portnum: PortNum::TracerouteApp as i32,
                                            request_id,
                                            payload: encode(&RouteDiscovery {
                                                route: vec![relay],
                                                snr_towards: vec![8, -20],
                                                route_back: vec![relay],
                                                snr_back: vec![-4, -12],
                                            }),
                                            ..Default::default()
                                        },
                                    )),
                                    ..Default::default()
                                },
                            )))
                            .await;
                    });
                }
            }
            PortNum::AdminApp => {
                let admin = AdminMessage::decode(data.payload.as_slice()).ok();
                match admin.and_then(|a| a.payload_variant) {
                    Some(admin_message::PayloadVariant::GetOwnerRequest(_)) => {
                        let _ = evt_tx
                            .send(TransportEvent::FromRadio(packet_to_from_radio(
                                MeshPacket {
                                    from: MY_NODE_NUM,
                                    to: 0,
                                    id: 0,
                                    rx_time: now_unix(),
                                    payload_variant: Some(mesh_packet::PayloadVariant::Decoded(
                                        Data {
                                            portnum: PortNum::AdminApp as i32,
                                            payload: encode(&AdminMessage {
                                                payload_variant: Some(
                                                    admin_message::PayloadVariant::GetOwnerResponse(
                                                        my_user(),
                                                    ),
                                                ),
                                                ..Default::default()
                                            }),
                                            ..Default::default()
                                        },
                                    )),
                                    ..Default::default()
                                },
                            )))
                            .await;
                    }
                    Some(admin_message::PayloadVariant::GetDeviceMetadataRequest(_)) => {
                        let _ = evt_tx
                            .send(TransportEvent::FromRadio(FromRadio {
                                id: 0,
                                payload_variant: Some(from_radio::PayloadVariant::Metadata(
                                    meshtastic_protobufs::meshtastic::DeviceMetadata {
                                        firmware_version: FIRMWARE_VERSION.to_string(),
                                        device_state_version: 23,
                                        can_shutdown: true,
                                        ..Default::default()
                                    },
                                )),
                            }))
                            .await;
                    }
                    Some(admin_message::PayloadVariant::RemoveByNodenum(num)) => {
                        removed.insert(num);
                    }
                    _ => {}
                }
            }
            PortNum::PositionApp if data.want_response => {
                // Someone asked where we are.
                let _ = evt_tx
                    .send(TransportEvent::FromRadio(packet_to_from_radio(
                        MeshPacket {
                            from: MY_NODE_NUM,
                            to: p.from,
                            id: 0,
                            rx_time: now_unix(),
                            channel: p.channel,
                            payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
                                portnum: PortNum::PositionApp as i32,
                                payload: encode(&Position {
                                    latitude_i: Some(40_015_100),
                                    longitude_i: Some(-105_280_500),
                                    altitude: Some(1_655),
                                    time: now_unix(),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            })),
                            ..Default::default()
                        },
                    )))
                    .await;
            }
            _ => {}
        }
    }
}

fn encode<M: Message>(msg: &M) -> Vec<u8> {
    mt_protocol::frame::encode_protobuf(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshtastic_protobufs::meshtastic::{PortNum, to_radio};

    #[tokio::test]
    async fn remove_by_nodenum_drops_the_node_from_later_downloads() {
        let (tx, mut rx) = mpsc::channel(128);
        let mut removed = HashSet::new();
        let forget = 0x7A11_0001; // Base Camp

        let admin = AdminMessage {
            payload_variant: Some(admin_message::PayloadVariant::RemoveByNodenum(forget)),
            ..Default::default()
        };
        let packet = MeshPacket {
            from: 0,
            to: BROADCAST_ADDR,
            id: 1,
            payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
                portnum: PortNum::AdminApp as i32,
                payload: encode(&admin),
                ..Default::default()
            })),
            ..Default::default()
        };
        handle_toradio(
            ToRadio {
                payload_variant: Some(to_radio::PayloadVariant::Packet(packet)),
            },
            &tx,
            &mut removed,
        )
        .await;
        assert!(removed.contains(&forget), "removal should be recorded");

        while rx.try_recv().is_ok() {}

        // A later Stage 2 download must not include the forgotten node.
        handle_toradio(
            ToRadio {
                payload_variant: Some(to_radio::PayloadVariant::WantConfigId(HANDSHAKE_NONCE_2)),
            },
            &tx,
            &mut removed,
        )
        .await;

        let mut names = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if let TransportEvent::FromRadio(FromRadio {
                payload_variant: Some(from_radio::PayloadVariant::NodeInfo(node)),
                ..
            }) = event
            {
                if let Some(user) = node.user {
                    names.push(user.long_name);
                }
            }
        }

        assert!(!names.iter().any(|name| name == "Base Camp"));
        assert!(names.iter().any(|name| name == "Ridge Repeater"));
    }
}
