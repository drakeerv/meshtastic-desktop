//! Schema definition, connection tuning and protobuf blob codecs.

use prost::Message;
use rusqlite::Connection;

use crate::{PersistenceError, Result};

/// Current schema version. Bump together with a migration step.
pub const SCHEMA_VERSION: i64 = 1;

/// Apply connection-level pragmas: WAL for concurrent readers, foreign keys
/// for integrity, and a busy timeout so bursts of writes never fail hard.
pub fn configure(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

/// Create tables and indices, stamping the schema version.
pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS meta (
            key        TEXT PRIMARY KEY NOT NULL,
            value      TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS nodes (
            num                 INTEGER PRIMARY KEY NOT NULL,
            user_id             TEXT,
            long_name           TEXT,
            short_name          TEXT,
            hw_model            INTEGER,
            role                INTEGER,
            is_licensed         INTEGER,
            snr                 REAL,
            last_heard          INTEGER,
            channel             INTEGER,
            via_mqtt            INTEGER,
            hops_away           INTEGER,
            is_favorite         INTEGER NOT NULL DEFAULT 0,
            is_ignored          INTEGER NOT NULL DEFAULT 0,
            latitude_i          INTEGER,
            longitude_i         INTEGER,
            altitude            INTEGER,
            position_time       INTEGER,
            battery_level       INTEGER,
            voltage             REAL,
            channel_utilization REAL,
            air_util_tx         REAL,
            uptime_seconds      INTEGER,
            user_blob           BLOB,
            position_blob       BLOB,
            node_info_blob      BLOB NOT NULL,
            updated_at          INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_nodes_last_heard ON nodes (last_heard DESC);
        CREATE INDEX IF NOT EXISTS idx_nodes_names      ON nodes (long_name, short_name);

        CREATE TABLE IF NOT EXISTS channels (
            idx          INTEGER PRIMARY KEY NOT NULL,
            name         TEXT,
            role         INTEGER,
            psk          BLOB,
            channel_blob BLOB NOT NULL,
            updated_at   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS configs (
            section     TEXT PRIMARY KEY NOT NULL,
            config_blob BLOB NOT NULL,
            updated_at  INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS module_configs (
            section     TEXT PRIMARY KEY NOT NULL,
            config_blob BLOB NOT NULL,
            updated_at  INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS messages (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            packet_id   INTEGER NOT NULL,
            channel     INTEGER NOT NULL,
            from_num    INTEGER NOT NULL,
            to_num      INTEGER NOT NULL,
            portnum     INTEGER NOT NULL,
            text        TEXT NOT NULL DEFAULT '',
            sent_at     INTEGER NOT NULL,
            received_at INTEGER NOT NULL,
            status      TEXT NOT NULL,
            is_outgoing INTEGER NOT NULL,
            want_ack    INTEGER NOT NULL DEFAULT 0,
            reply_id    INTEGER NOT NULL DEFAULT 0,
            rx_snr      REAL,
            rx_rssi     INTEGER,
            hop_start   INTEGER,
            hop_limit   INTEGER,
            error       TEXT,
            packet_blob BLOB,
            UNIQUE (packet_id, is_outgoing)
        );
        CREATE INDEX IF NOT EXISTS idx_messages_channel ON messages (channel, id);
        CREATE INDEX IF NOT EXISTS idx_messages_from    ON messages (from_num, id);
        CREATE INDEX IF NOT EXISTS idx_messages_to      ON messages (to_num, id);
        CREATE INDEX IF NOT EXISTS idx_messages_status  ON messages (status);

        CREATE TABLE IF NOT EXISTS telemetry (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            node_num       INTEGER NOT NULL,
            kind           TEXT NOT NULL,
            timestamp      INTEGER NOT NULL,
            telemetry_blob BLOB NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_telemetry_node ON telemetry (node_num, id DESC);

        CREATE TABLE IF NOT EXISTS positions (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            node_num      INTEGER NOT NULL,
            timestamp     INTEGER NOT NULL,
            latitude_i    INTEGER,
            longitude_i   INTEGER,
            altitude      INTEGER,
            position_blob BLOB NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_positions_node ON positions (node_num, id DESC);
        "#,
    )?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

/// Encode a prost message for storage.
pub(crate) fn encode<M: Message>(msg: &M) -> Vec<u8> {
    mt_protocol::frame::encode_protobuf(msg)
}

/// Decode a stored protobuf blob.
pub(crate) fn decode<M: Message + Default>(bytes: &[u8]) -> Result<M> {
    M::decode(bytes).map_err(|e| PersistenceError::Protobuf(e.to_string()))
}

/// The section name a `Config` variant maps to, used as a primary key so
/// repeated config downloads replace the previous values in place.
pub(crate) fn config_section(config: &meshtastic_protobufs::meshtastic::Config) -> &'static str {
    use meshtastic_protobufs::meshtastic::config::PayloadVariant as V;
    match config.payload_variant {
        Some(V::Device(_)) => "device",
        Some(V::Position(_)) => "position",
        Some(V::Power(_)) => "power",
        Some(V::Network(_)) => "network",
        Some(V::Display(_)) => "display",
        Some(V::Lora(_)) => "lora",
        Some(V::Bluetooth(_)) => "bluetooth",
        Some(V::Security(_)) => "security",
        Some(V::Sessionkey(_)) => "sessionkey",
        Some(V::DeviceUi(_)) => "device_ui",
        None => "unknown",
    }
}

/// The section name a `ModuleConfig` variant maps to.
pub(crate) fn module_config_section(
    config: &meshtastic_protobufs::meshtastic::ModuleConfig,
) -> &'static str {
    use meshtastic_protobufs::meshtastic::module_config::PayloadVariant as V;
    match config.payload_variant {
        Some(V::Mqtt(_)) => "mqtt",
        Some(V::Serial(_)) => "serial",
        Some(V::ExternalNotification(_)) => "external_notification",
        Some(V::StoreForward(_)) => "store_forward",
        Some(V::RangeTest(_)) => "range_test",
        Some(V::Telemetry(_)) => "telemetry",
        Some(V::CannedMessage(_)) => "canned_message",
        Some(V::Audio(_)) => "audio",
        Some(V::RemoteHardware(_)) => "remote_hardware",
        Some(V::NeighborInfo(_)) => "neighbor_info",
        Some(V::AmbientLighting(_)) => "ambient_lighting",
        Some(V::DetectionSensor(_)) => "detection_sensor",
        Some(V::Paxcounter(_)) => "paxcounter",
        None => "unknown",
    }
}
