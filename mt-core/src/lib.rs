//! # mt-core
//!
//! The UI-independent heart of the application: a single actor that owns
//! the connection lifecycle, drives the Meshtastic config handshake,
//! ingests `FromRadio` traffic into memory and the per-device database, and
//! tracks outbound message delivery.
//!
//! The front end interacts only through [`CoreHandle`] (dispatch a
//! [`CoreCommand`], subscribe to [`CoreEvent`]s); it never sees transports,
//! protobuf plumbing or SQLite.

pub mod events;
mod ingest;
mod outbound;
mod state;
mod supervisor;

pub use events::{ConnectionState, CoreCommand, CoreEvent};
pub use supervisor::CoreConfig;

// Re-export the message vocabulary so the UI depends on one crate.
pub use mt_persistence::{MessageFilter, MessageQuery, MessageRecord, MessageStatus};
pub use mt_transport::{DeviceAddress, DiscoveredDevice, DiscoveryEvent, TransportKind};

use tokio::sync::{broadcast, mpsc};

/// Errors surfaced by the core API.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("the core is not running")]
    Closed,
    #[error("not connected to a node")]
    NotConnected,
    #[error("invalid command: {0}")]
    Invalid(String),
    #[error("persistence error: {0}")]
    Persistence(#[from] mt_persistence::PersistenceError),
    #[error("transport error: {0}")]
    Transport(#[from] mt_transport::TransportError),
}

pub type Result<T> = std::result::Result<T, CoreError>;

/// Cloneable front-end handle to the core actor.
#[derive(Clone)]
pub struct CoreHandle {
    cmd: mpsc::Sender<CoreCommand>,
    events: broadcast::Sender<CoreEvent>,
}

impl CoreHandle {
    /// Dispatch a command, waiting if the core's queue is full.
    pub async fn dispatch(&self, cmd: CoreCommand) -> Result<()> {
        self.cmd.send(cmd).await.map_err(|_| CoreError::Closed)
    }

    /// Dispatch a command without waiting.
    pub fn try_dispatch(&self, cmd: CoreCommand) -> Result<()> {
        self.cmd.try_send(cmd).map_err(|err| match err {
            mpsc::error::TrySendError::Closed(_) => CoreError::Closed,
            mpsc::error::TrySendError::Full(_) => CoreError::Invalid("core queue full".into()),
        })
    }

    /// Subscribe to the core event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<CoreEvent> {
        self.events.subscribe()
    }

    /// Whether the core actor has exited.
    pub fn is_closed(&self) -> bool {
        self.cmd.is_closed()
    }

    /// Convenience: connect to a device.
    pub async fn connect(&self, address: DeviceAddress) -> Result<()> {
        self.dispatch(CoreCommand::Connect(address)).await
    }

    /// Convenience: gracefully disconnect.
    pub async fn disconnect(&self) -> Result<()> {
        self.dispatch(CoreCommand::Disconnect).await
    }
}

/// Spawn the core actor. Must be called from within a tokio runtime.
pub fn spawn_core(config: CoreConfig) -> CoreHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel(256);
    let (event_tx, _) = broadcast::channel(2048);
    let supervisor = supervisor::Supervisor::new(config, event_tx.clone(), cmd_rx);
    tokio::spawn(supervisor.run());
    CoreHandle {
        cmd: cmd_tx,
        events: event_tx,
    }
}
