//! End-to-end test of the core actor against the in-process mock radio.
//!
//! This exercises the real handshake, ingest, persistence and outbound
//! acknowledgement paths without any hardware.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mt_core::{
    ConnectionState, CoreCommand, CoreConfig, CoreEvent, DeviceAddress, MessageStatus, spawn_core,
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
