//! End-to-end test of the core actor against the in-process mock radio.
//!
//! This exercises the real handshake, ingest, persistence and outbound
//! acknowledgement paths without any hardware.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mt_core::{
    ConnectionState, CoreCommand, CoreConfig, CoreEvent, DeviceAddress, MessageFilter,
    MessageStatus, spawn_core,
};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::timeout;

fn temp_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mt-core-test-{nanos}"))
}

/// State observed while waiting for the handshake to complete.
#[derive(Default)]
struct Handshake {
    node_num: u32,
    my_info: bool,
    channel: bool,
    messages_loaded: bool,
    node_names: Vec<String>,
    node_nums: Vec<u32>,
}

/// Drive the event stream until the connection reaches `Connected`,
/// recording everything seen on the way.
async fn wait_connected(rx: &mut tokio::sync::broadcast::Receiver<CoreEvent>) -> Handshake {
    timeout(Duration::from_secs(10), async {
        let mut seen = Handshake::default();
        loop {
            match rx.recv().await {
                Ok(CoreEvent::Connection(ConnectionState::Connected { node_num, .. })) => {
                    seen.node_num = node_num;
                    return seen;
                }
                Ok(CoreEvent::MyInfo(_)) => seen.my_info = true,
                Ok(CoreEvent::Channel(_)) => seen.channel = true,
                Ok(CoreEvent::MessagesLoaded(_)) => seen.messages_loaded = true,
                Ok(CoreEvent::Node(node)) => {
                    if let Some(user) = &node.user {
                        seen.node_names.push(user.long_name.clone());
                    }
                    if !seen.node_nums.contains(&node.num) {
                        seen.node_nums.push(node.num);
                    }
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => panic!("core event stream closed"),
            }
        }
    })
    .await
    .expect("handshake never completed")
}

#[tokio::test]
async fn mock_handshake_ingest_and_messaging() {
    let dir = temp_dir();
    let cfg = CoreConfig {
        data_dir: dir.clone(),
        handshake_timeout: Duration::from_secs(5),
        message_timeout: Duration::from_secs(10),
        ..CoreConfig::default()
    };
    let core = spawn_core(cfg);
    let mut events = core.subscribe();

    core.connect(DeviceAddress::mock("test")).await.unwrap();
    let handshake = wait_connected(&mut events).await;

    assert_eq!(handshake.node_num, 0x0042_A1B2, "mock local node number");
    assert!(handshake.my_info, "my info should be delivered");
    assert!(handshake.channel, "channels should be delivered");
    assert!(
        handshake.messages_loaded,
        "history snapshot should be delivered"
    );
    assert!(
        handshake.node_names.iter().any(|n| n == "Base Camp"),
        "cast node should be present: {:?}",
        handshake.node_names
    );
    assert!(
        handshake.node_names.iter().any(|n| n == "Mock Station"),
        "own node should be present: {:?}",
        handshake.node_names
    );

    // The mock ACKs a direct message twice: first an implicit (mesh) ACK
    // from the local node when a neighbour rebroadcasts it, then the
    // destination's real ACK. Only the latter may count as delivered, and
    // hearing the destination's ACK must refresh its last-heard time.
    let dm_target = handshake
        .node_nums
        .iter()
        .copied()
        .find(|num| *num != handshake.node_num)
        .expect("a peer to message");
    core.dispatch(CoreCommand::SendText {
        text: "direct hello".into(),
        channel: 0,
        to: Some(dm_target),
        reply_id: None,
    })
    .await
    .unwrap();
    let (statuses, heard) = timeout(Duration::from_secs(5), async {
        let mut statuses = Vec::new();
        let mut heard = None;
        loop {
            match events.recv().await {
                Ok(CoreEvent::Node(node)) if node.num == dm_target && node.last_heard > 0 => {
                    heard = Some(node.last_heard);
                }
                Ok(CoreEvent::MessageStatus { status, .. })
                    if matches!(status, MessageStatus::Delivered | MessageStatus::Received) =>
                {
                    statuses.push(status);
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => panic!("core event stream closed"),
            }
            if statuses.last() == Some(&MessageStatus::Received)
                && let Some(heard) = heard
            {
                break (statuses, heard);
            }
        }
    })
    .await
    .expect("direct message was never acknowledged by the destination");

    assert_eq!(
        statuses,
        vec![MessageStatus::Delivered, MessageStatus::Received],
        "mesh ACK first, destination ACK upgrades the status"
    );
    assert!(
        heard > 0,
        "destination ACK should refresh the peer's last heard"
    );

    // Send a channel message; the mock answers with a routing ack.
    core.dispatch(CoreCommand::SendText {
        text: "hello from the test".into(),
        channel: 0,
        to: None,
        reply_id: None,
    })
    .await
    .unwrap();

    let delivered = timeout(Duration::from_secs(15), async {
        let mut stored = false;
        let mut enroute = false;
        loop {
            match events.recv().await {
                Ok(CoreEvent::Message(record)) => {
                    if record.outgoing && record.text == "hello from the test" {
                        stored = true;
                    }
                }
                Ok(CoreEvent::MessageStatus {
                    status: MessageStatus::Enroute,
                    ..
                }) => enroute = true,
                Ok(CoreEvent::MessageStatus {
                    status: MessageStatus::Delivered,
                    ..
                }) => return (stored, enroute),
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => panic!("core event stream closed"),
            }
        }
    })
    .await
    .expect("message was never acknowledged");

    assert!(delivered.0, "outgoing message should be stored immediately");
    assert!(delivered.1, "message should pass through Enroute");

    // A traceroute response only carries intermediate hops on the wire; the
    // core rebuilds the full path with both endpoints for each direction.
    core.dispatch(CoreCommand::Traceroute(dm_target))
        .await
        .unwrap();
    let traceroute = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(CoreEvent::Traceroute {
                target,
                route,
                snr_towards,
                route_back,
                snr_back,
                ..
            }) = events.recv().await
            {
                return (target, route, snr_towards, route_back, snr_back);
            }
        }
    })
    .await
    .expect("traceroute response was never emitted");

    assert_eq!(traceroute.0, dm_target, "target is the responding node");
    assert_eq!(
        traceroute.1,
        vec![handshake.node_num, 0x0BAD_CAFE, dm_target],
        "forward route includes origin and target"
    );
    assert_eq!(traceroute.2, vec![8, -20]);
    assert_eq!(
        traceroute.3,
        vec![dm_target, 0x0BAD_CAFE, handshake.node_num],
        "return route includes target and origin"
    );
    assert_eq!(traceroute.4, vec![-4, -12]);

    // Removing a node drops it locally and tells the device to forget it.
    let target = handshake
        .node_nums
        .iter()
        .copied()
        .find(|num| *num != handshake.node_num)
        .expect("a cast node to remove");
    core.dispatch(CoreCommand::RemoveNode(target))
        .await
        .unwrap();
    let removed = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(CoreEvent::NodeRemoved(num)) = events.recv().await {
                if num == target {
                    return;
                }
            }
        }
    })
    .await;
    assert!(removed.is_ok(), "removing a node should emit NodeRemoved");

    // Renaming the owner updates the local node immediately (no reconnect).
    core.dispatch(CoreCommand::SetOwner {
        long_name: "New Base Camp".into(),
        short_name: "NBC".into(),
        is_licensed: false,
    })
    .await
    .unwrap();
    let renamed = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(CoreEvent::Node(node)) = events.recv().await {
                if node.num == handshake.node_num {
                    if let Some(user) = &node.user {
                        if user.long_name == "New Base Camp" && user.short_name == "NBC" {
                            return;
                        }
                    }
                }
            }
        }
    })
    .await;
    assert!(renamed.is_ok(), "owner rename should update the local node");

    // Importing a shared contact adds the node locally with its public key,
    // which is what makes PKC direct messages to it possible.
    let contact_num = 0x00AB_CDEF;
    let contact = meshtastic_protobufs::meshtastic::SharedContact {
        node_num: contact_num,
        user: Some(meshtastic_protobufs::meshtastic::User {
            id: "!00abcdef".into(),
            long_name: "Imported Contact".into(),
            short_name: "IC".into(),
            public_key: vec![0x11; 32],
            ..Default::default()
        }),
        should_ignore: false,
    };
    core.dispatch(CoreCommand::AddContact(Box::new(contact)))
        .await
        .unwrap();
    let imported = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(CoreEvent::Node(node)) = events.recv().await {
                if node.num == contact_num {
                    if let Some(user) = &node.user {
                        if user.public_key == vec![0x11u8; 32] {
                            return;
                        }
                    }
                }
            }
        }
    })
    .await;
    assert!(
        imported.is_ok(),
        "importing a contact should add the node with its key"
    );

    // And the database should hold the delivered message.
    let db = mt_persistence::Database::open_for_device(&dir, handshake.node_num).unwrap();
    let history = db
        .list_messages(&mt_persistence::MessageQuery::channel(0, 50))
        .unwrap();
    assert!(
        history
            .iter()
            .any(|m| m.text == "hello from the test" && m.status == MessageStatus::Delivered)
    );

    core.dispatch(CoreCommand::ShutdownCore).await.unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

/// Wait for a `MessagesLoaded` snapshot satisfying `predicate`.
async fn wait_history(
    rx: &mut tokio::sync::broadcast::Receiver<CoreEvent>,
    predicate: impl Fn(&[mt_core::MessageRecord]) -> bool,
) -> Vec<mt_core::MessageRecord> {
    timeout(Duration::from_secs(5), async {
        loop {
            match rx.recv().await {
                Ok(CoreEvent::MessagesLoaded(messages)) if predicate(&messages) => {
                    return messages;
                }
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => panic!("core event stream closed"),
            }
        }
    })
    .await
    .expect("expected a history snapshot")
}

/// Loading history while idle, then clearing and deleting messages, must all
/// be persisted to the per-device database.
#[tokio::test]
async fn mock_history_load_clear_and_delete() {
    let dir = temp_dir();
    let cfg = CoreConfig {
        data_dir: dir.clone(),
        handshake_timeout: Duration::from_secs(5),
        ..CoreConfig::default()
    };
    let core = spawn_core(cfg);
    let mut events = core.subscribe();

    core.connect(DeviceAddress::mock("test")).await.unwrap();
    let handshake = wait_connected(&mut events).await;
    let node_num = handshake.node_num;

    // Sending a channel message stores it locally.
    core.dispatch(CoreCommand::SendText {
        text: "clear me".into(),
        channel: 0,
        to: None,
        reply_id: None,
    })
    .await
    .unwrap();
    let stored_id = timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(CoreEvent::Message(record)) = events.recv().await {
                if record.outgoing && record.text == "clear me" {
                    return record.id;
                }
            }
        }
    })
    .await
    .expect("outgoing message should be stored");

    // Loading history re-reads the same snapshot (this is the offline path).
    core.dispatch(CoreCommand::LoadHistory(node_num))
        .await
        .unwrap();
    wait_history(&mut events, |messages| {
        messages.iter().any(|m| m.id == stored_id)
    })
    .await;

    // Clearing the channel drops it from the database and the UI.
    core.dispatch(CoreCommand::ClearConversation(Box::new(
        MessageFilter::Channel(0),
    )))
    .await
    .unwrap();
    let cleared = wait_history(&mut events, |messages| {
        messages.iter().all(|m| m.id != stored_id)
    })
    .await;
    assert!(cleared.iter().all(|m| m.id != stored_id));

    // A new message can then be deleted individually.
    core.dispatch(CoreCommand::SendText {
        text: "delete me".into(),
        channel: 0,
        to: None,
        reply_id: None,
    })
    .await
    .unwrap();
    let delete_id = timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(CoreEvent::Message(record)) = events.recv().await {
                if record.outgoing && record.text == "delete me" {
                    return record.id;
                }
            }
        }
    })
    .await
    .expect("second outgoing message should be stored");

    core.dispatch(CoreCommand::DeleteMessage(delete_id))
        .await
        .unwrap();
    let after_delete = wait_history(&mut events, |messages| {
        messages.iter().all(|m| m.id != delete_id)
    })
    .await;
    assert!(after_delete.iter().all(|m| m.id != delete_id));

    // The database itself is empty afterwards.
    let db = mt_persistence::Database::open_for_device(&dir, node_num).unwrap();
    let remaining = db
        .list_messages(&mt_persistence::MessageQuery::channel(0, 50))
        .unwrap();
    assert!(
        remaining.is_empty(),
        "clear + delete should empty the channel: {remaining:?}"
    );

    core.dispatch(CoreCommand::ShutdownCore).await.unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
