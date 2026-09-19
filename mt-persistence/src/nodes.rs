//! Node database: one row per known mesh node.
//!
//! The full `NodeInfo` protobuf is the source of truth; scalar columns are
//! denormalised out of it purely so the UI can sort and search without
//! decoding every row.

use meshtastic_protobufs::meshtastic::NodeInfo;
use rusqlite::{OptionalExtension, params};

use crate::schema::{decode, encode};
use crate::{Database, Result, now_unix};

impl Database {
    /// Insert or update a node.
    ///
    /// Fields absent from `info` (no `user`, no `position`, no
    /// `device_metrics`) keep their previously stored values instead of
    /// being cleared, so partial updates from live packets never erase
    /// data learned during the handshake.
    pub fn upsert_node(&self, info: &NodeInfo) -> Result<()> {
        self.with_conn(|conn| {
            // Merge with the stored row so absent fields survive. The
            // separate columns below could do this with COALESCE, but the
            // canonical `node_info_blob` must be merged too or reads would
            // lose data the columns still hold.
            let existing = conn
                .query_row(
                    "SELECT node_info_blob FROM nodes WHERE num = ?1",
                    params![info.num],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()?;
            let merged = match existing {
                Some(bytes) => merge_node_info(decode(&bytes)?, info),
                None => info.clone(),
            };

            let user = merged.user.as_ref();
            let position = merged.position.as_ref();
            let metrics = merged.device_metrics.as_ref();
            let user_blob = merged.user.as_ref().map(encode);
            let position_blob = merged.position.as_ref().map(encode);
            let node_blob = encode(&merged);

            conn.execute(
                "INSERT INTO nodes (
                     num, user_id, long_name, short_name, hw_model, role, is_licensed,
                     snr, last_heard, channel, via_mqtt, hops_away, is_favorite, is_ignored,
                     latitude_i, longitude_i, altitude, position_time,
                     battery_level, voltage, channel_utilization, air_util_tx, uptime_seconds,
                     user_blob, position_blob, node_info_blob, updated_at
                 ) VALUES (
                     ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                     ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                     ?15, ?16, ?17, ?18,
                     ?19, ?20, ?21, ?22, ?23,
                     ?24, ?25, ?26, ?27
                 )
                 ON CONFLICT(num) DO UPDATE SET
                     user_id             = COALESCE(excluded.user_id, nodes.user_id),
                     long_name           = COALESCE(excluded.long_name, nodes.long_name),
                     short_name          = COALESCE(excluded.short_name, nodes.short_name),
                     hw_model            = COALESCE(excluded.hw_model, nodes.hw_model),
                     role                = COALESCE(excluded.role, nodes.role),
                     is_licensed         = COALESCE(excluded.is_licensed, nodes.is_licensed),
                     snr                 = excluded.snr,
                     last_heard          = excluded.last_heard,
                     channel             = excluded.channel,
                     via_mqtt            = excluded.via_mqtt,
                     hops_away           = COALESCE(excluded.hops_away, nodes.hops_away),
                     is_favorite         = excluded.is_favorite,
                     is_ignored          = excluded.is_ignored,
                     latitude_i          = COALESCE(excluded.latitude_i, nodes.latitude_i),
                     longitude_i         = COALESCE(excluded.longitude_i, nodes.longitude_i),
                     altitude            = COALESCE(excluded.altitude, nodes.altitude),
                     position_time       = COALESCE(excluded.position_time, nodes.position_time),
                     battery_level       = COALESCE(excluded.battery_level, nodes.battery_level),
                     voltage             = COALESCE(excluded.voltage, nodes.voltage),
                     channel_utilization = COALESCE(excluded.channel_utilization, nodes.channel_utilization),
                     air_util_tx         = COALESCE(excluded.air_util_tx, nodes.air_util_tx),
                     uptime_seconds      = COALESCE(excluded.uptime_seconds, nodes.uptime_seconds),
                     user_blob           = COALESCE(excluded.user_blob, nodes.user_blob),
                     position_blob       = COALESCE(excluded.position_blob, nodes.position_blob),
                     node_info_blob      = excluded.node_info_blob,
                     updated_at          = excluded.updated_at",
                params![
                    merged.num,
                    user.map(|u| u.id.as_str()),
                    user.map(|u| u.long_name.as_str()),
                    user.map(|u| u.short_name.as_str()),
                    user.map(|u| u.hw_model),
                    user.map(|u| u.role),
                    user.map(|u| u.is_licensed),
                    merged.snr,
                    merged.last_heard as i64,
                    merged.channel,
                    merged.via_mqtt,
                    merged.hops_away.map(|h| h as i64),
                    merged.is_favorite,
                    merged.is_ignored,
                    position.and_then(|p| p.latitude_i),
                    position.and_then(|p| p.longitude_i),
                    position.and_then(|p| p.altitude),
                    position.map(|p| p.time as i64),
                    metrics.and_then(|m| m.battery_level),
                    metrics.and_then(|m| m.voltage),
                    metrics.and_then(|m| m.channel_utilization),
                    metrics.and_then(|m| m.air_util_tx),
                    metrics.and_then(|m| m.uptime_seconds),
                    user_blob,
                    position_blob,
                    node_blob,
                    now_unix(),
                ],
            )?;
            Ok(())
        })
    }

    /// All nodes, favourites first, then most recently heard.
    pub fn list_nodes(&self) -> Result<Vec<NodeInfo>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT node_info_blob FROM nodes
                 ORDER BY is_favorite DESC, last_heard DESC, long_name COLLATE NOCASE ASC",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
            let mut nodes = Vec::new();
            for row in rows {
                nodes.push(decode::<NodeInfo>(&row?)?);
            }
            Ok(nodes)
        })
    }

    /// Fetch a single node by number.
    pub fn get_node(&self, num: u32) -> Result<Option<NodeInfo>> {
        self.with_conn(|conn| {
            let blob = conn
                .query_row(
                    "SELECT node_info_blob FROM nodes WHERE num = ?1",
                    params![num],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()?;
            match blob {
                Some(bytes) => Ok(Some(decode(&bytes)?)),
                None => Ok(None),
            }
        })
    }

    /// Remove a node from the database.
    pub fn remove_node(&self, num: u32) -> Result<usize> {
        self.with_conn(|conn| Ok(conn.execute("DELETE FROM nodes WHERE num = ?1", params![num])?))
    }

    /// Mark (or unmark) a node as a favourite. Kept in sync with the
    /// `NodeInfo` blob so reads reflect the change immediately.
    pub fn set_favorite(&self, num: u32, favorite: bool) -> Result<usize> {
        self.set_node_flag(num, "is_favorite", favorite)
    }

    /// Mark (or unmark) a node as ignored.
    pub fn set_ignored(&self, num: u32, ignored: bool) -> Result<usize> {
        self.set_node_flag(num, "is_ignored", ignored)
    }

    fn set_node_flag(&self, num: u32, column: &'static str, value: bool) -> Result<usize> {
        // `column` is a compile-time constant chosen by our own methods, so
        // interpolating it cannot enable injection.
        let sql = format!("UPDATE nodes SET {column} = ?1, updated_at = ?2 WHERE num = ?3");
        self.with_tx(|tx| {
            let changed = tx.execute(&sql, params![value, now_unix(), num])?;
            if changed > 0 {
                let blob: Vec<u8> = tx.query_row(
                    "SELECT node_info_blob FROM nodes WHERE num = ?1",
                    params![num],
                    |row| row.get(0),
                )?;
                let mut info: NodeInfo = decode(&blob)?;
                match column {
                    "is_favorite" => info.is_favorite = value,
                    "is_ignored" => info.is_ignored = value,
                    _ => {}
                }
                tx.execute(
                    "UPDATE nodes SET node_info_blob = ?1 WHERE num = ?2",
                    params![encode(&info), num],
                )?;
            }
            Ok(changed)
        })
    }

    /// Number of nodes currently stored.
    pub fn node_count(&self) -> Result<u32> {
        self.with_conn(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))?)
        })
    }

    /// Delete all nodes (used when the device reports a nodedb reset).
    pub fn clear_nodes(&self) -> Result<usize> {
        self.with_conn(|conn| Ok(conn.execute("DELETE FROM nodes", [])?))
    }
}

/// Fill optional fields absent from `incoming` with the values already
/// stored, so a partial live update never erases handshake data.
fn merge_node_info(existing: NodeInfo, incoming: &NodeInfo) -> NodeInfo {
    let mut merged = incoming.clone();
    if merged.user.is_none() {
        merged.user = existing.user;
    }
    if merged.position.is_none() {
        merged.position = existing.position;
    }
    if merged.device_metrics.is_none() {
        merged.device_metrics = existing.device_metrics;
    }
    if merged.hops_away.is_none() {
        merged.hops_away = existing.hops_away;
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshtastic_protobufs::meshtastic::{DeviceMetrics, Position, User};

    fn sample(num: u32, name: &str) -> NodeInfo {
        NodeInfo {
            num,
            user: Some(User {
                id: format!("!{num:08x}"),
                long_name: name.into(),
                short_name: name[..4.min(name.len())].into(),
                hw_model: 32,
                ..Default::default()
            }),
            position: Some(Position {
                latitude_i: Some(40_000_000),
                longitude_i: Some(-105_000_000),
                altitude: Some(1_600),
                time: 1_700_000_000,
                ..Default::default()
            }),
            snr: -8.5,
            last_heard: 1_700_000_100,
            hops_away: Some(2),
            device_metrics: Some(DeviceMetrics {
                battery_level: Some(88),
                voltage: Some(4.11),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn upsert_and_fetch_round_trip() {
        let db = Database::open_in_memory().unwrap();
        let node = sample(0x1234_5678, "Base Camp");
        db.upsert_node(&node).unwrap();

        let all = db.list_nodes().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].num, node.num);
        assert_eq!(all[0].user.as_ref().unwrap().long_name, "Base Camp");
        assert_eq!(all[0].position.as_ref().unwrap().altitude, Some(1_600));
        assert_eq!(db.get_node(node.num).unwrap().unwrap(), node);
    }

    #[test]
    fn partial_update_preserves_user_and_position() {
        let db = Database::open_in_memory().unwrap();
        let mut node = sample(0x1111, "Ridge");
        db.upsert_node(&node).unwrap();

        // A live packet update carrying only signal info.
        node.user = None;
        node.position = None;
        node.device_metrics = None;
        node.snr = -3.0;
        node.last_heard = 1_700_000_500;
        db.upsert_node(&node).unwrap();

        let stored = db.get_node(0x1111).unwrap().unwrap();
        assert_eq!(stored.user.unwrap().long_name, "Ridge");
        assert!(stored.position.is_some());
        assert_eq!(stored.snr, -3.0);
        assert_eq!(stored.last_heard, 1_700_000_500);
        assert_eq!(db.node_count().unwrap(), 1);
    }

    #[test]
    fn favourites_sort_first() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_node(&sample(1, "Alpha")).unwrap();
        db.upsert_node(&sample(2, "Bravo")).unwrap();
        db.upsert_node(&sample(3, "Charlie")).unwrap();
        db.set_favorite(3, true).unwrap();

        let all = db.list_nodes().unwrap();
        assert_eq!(all[0].num, 3);
        assert!(all[0].is_favorite);
        // Flag also reflected in the stored protobuf.
        assert!(db.get_node(3).unwrap().unwrap().is_favorite);
    }

    #[test]
    fn remove_and_clear() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_node(&sample(1, "Alpha")).unwrap();
        db.upsert_node(&sample(2, "Bravo")).unwrap();
        assert_eq!(db.remove_node(1).unwrap(), 1);
        assert_eq!(db.node_count().unwrap(), 1);
        assert_eq!(db.clear_nodes().unwrap(), 1);
        assert_eq!(db.node_count().unwrap(), 0);
    }
}
