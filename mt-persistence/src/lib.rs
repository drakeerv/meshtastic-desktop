//! # mt-persistence
//!
//! Per-device SQLite storage for a Meshtastic desktop client.
//!
//! Every node the user connects to gets its own database file
//! (`<data-dir>/devices/<node-num>.db`); the schema covers nodes, channels,
//! device/module configuration, messages, telemetry and position history,
//! plus a small key/value table for things like the advertised firmware
//! version.
//!
//! ## Storage strategy
//!
//! Rows keep the **raw protobuf blob** for every entity and, alongside it,
//! a handful of indexed columns used for sorting and filtering. Reads
//! decode the blob straight back into the original prost type, so no
//! hand-written mapping layer can drift from the protocol. Messages are the
//! exception: they carry a status that only exists client side, so they get
//! a dedicated [`MessageRecord`].
//!
//! The type is deliberately synchronous and cheap to clone: it is a handle
//! around a mutex-guarded `rusqlite::Connection`. Callers on an async
//! runtime should wrap bulk work in `tokio::task::spawn_blocking` when the
//! database is large.

pub mod channels;
pub mod config;
pub mod messages;
pub mod meta;
pub mod nodes;
pub mod schema;
pub mod telemetry;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

pub use messages::{MessageFilter, MessageQuery, MessageRecord, MessageStatus};
pub use telemetry::TelemetryKind;

/// Errors produced by the persistence layer.
#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no platform data directory available")]
    NoDataDir,
    #[error("protobuf codec error: {0}")]
    Protobuf(String),
    #[error("database mutex poisoned")]
    LockPoisoned,
    #[error("stored value for `{0}` could not be interpreted")]
    InvalidValue(&'static str),
}

pub type Result<T> = std::result::Result<T, PersistenceError>;

/// Handle to a per-device database. Clone freely; all clones share the
/// same connection.
#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("path", &self.path)
            .finish()
    }
}

impl Database {
    /// Open (creating if needed) the database at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        schema::configure(&conn)?;
        schema::migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: Some(path),
        })
    }

    /// An ephemeral in-memory database (used by tests and the demo mode).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::configure(&conn)?;
        schema::migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: None,
        })
    }

    /// Open the database for a specific device node number inside `dir`.
    pub fn open_for_device(dir: impl AsRef<Path>, node_num: u32) -> Result<Self> {
        Self::open(
            dir.as_ref()
                .join("devices")
                .join(format!("{node_num:08x}.db")),
        )
    }

    /// Default application data directory (`~/.local/share/meshtastic` on
    /// Linux).
    pub fn default_dir() -> Result<PathBuf> {
        let base = dirs::data_dir().ok_or(PersistenceError::NoDataDir)?;
        Ok(base.join("meshtastic"))
    }

    /// The file backing this database, if it is file-backed.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Run `f` with the raw connection held under the mutex.
    pub(crate) fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| PersistenceError::LockPoisoned)?;
        f(&guard)
    }

    /// Run `f` inside a transaction, committing on success.
    pub(crate) fn with_tx<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| PersistenceError::LockPoisoned)?;
        let tx = guard.unchecked_transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Remove every row from every table. Useful when re-provisioning a
    /// device or resetting a demo database.
    pub fn clear_all(&self) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute_batch(
                "DELETE FROM nodes;
                 DELETE FROM channels;
                 DELETE FROM configs;
                 DELETE FROM module_configs;
                 DELETE FROM messages;
                 DELETE FROM telemetry;
                 DELETE FROM positions;
                 DELETE FROM meta;",
            )?;
            Ok(())
        })
    }
}

/// Seconds since the Unix epoch, saturating at 0 for a pre-1970 clock.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
