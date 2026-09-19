//! Bridge between the tokio-based core/discovery tasks and iced's
//! subscription system.
//!
//! A single long-lived subscription multiplexes core events and discovery
//! snapshots into iced messages. Subscription data is keyed by a stable id
//! so iced keeps exactly one bridge stream alive.

use std::hash::{Hash, Hasher};

use iced::Subscription;
use iced::futures::{SinkExt, Stream};
use mt_core::CoreHandle;
use mt_transport::DiscoveryHandle;
use tokio::sync::broadcast::error::RecvError;

use crate::app::Message;

/// Holds the handles needed by the event bridge.
#[derive(Clone)]
pub struct Bridge {
    id: u64,
    core: CoreHandle,
    discovery: DiscoveryHandle,
}

impl Bridge {
    pub fn new(core: CoreHandle, discovery: DiscoveryHandle) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            core,
            discovery,
        }
    }

    /// The core handle, for dispatching commands.
    pub fn core(&self) -> &CoreHandle {
        &self.core
    }

    /// The discovery handle, for steering scans.
    pub fn discovery(&self) -> &DiscoveryHandle {
        &self.discovery
    }

    /// The subscription that feeds core and discovery events into the app.
    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::run_with(self.clone(), stream_bridge)
    }
}

impl Hash for Bridge {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Build the multiplexing stream. Must be a plain function so it coerces to
/// a `fn` pointer for `Subscription::run_with`.
fn stream_bridge(bridge: &Bridge) -> impl Stream<Item = Message> + use<> {
    let core = bridge.core.clone();
    let discovery = bridge.discovery.clone();

    iced::stream::channel(512, async move |mut output| {
        let mut core_rx = core.subscribe();
        let mut discovery_rx = discovery.subscribe();

        loop {
            tokio::select! {
                event = core_rx.recv() => match event {
                    Ok(event) => {
                        if output.send(Message::Core(event)).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
                event = discovery_rx.recv() => match event {
                    Ok(event) => {
                        if output.send(Message::Discovery(event)).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
            }
        }
    })
}
