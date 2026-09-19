//! Telemetry and position history.
//!
//! These are append-only time series; the UI reads the most recent page per
//! node. A `prune_before` helper keeps the databases bounded.

use meshtastic_protobufs::meshtastic::{Position, Telemetry, telemetry};
use rusqlite::params;

use crate::schema::{decode, encode};
use crate::{Database, Result, now_unix};

/// Which flavour of metrics a telemetry row carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryKind {
    Device,
    Environment,
    AirQuality,
    Power,
    LocalStats,
    Health,
    Host,
    Unknown,
}

impl TelemetryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TelemetryKind::Device => "device",
            TelemetryKind::Environment => "environment",
            TelemetryKind::AirQuality => "air_quality",
            TelemetryKind::Power => "power",
            TelemetryKind::LocalStats => "local_stats",
            TelemetryKind::Health => "health",
            TelemetryKind::Host => "host",
            TelemetryKind::Unknown => "unknown",
        }
    }

    pub fn of(telemetry: &Telemetry) -> Self {
        match telemetry.variant {
            Some(telemetry::Variant::DeviceMetrics(_)) => TelemetryKind::Device,
            Some(telemetry::Variant::EnvironmentMetrics(_)) => TelemetryKind::Environment,
            Some(telemetry::Variant::AirQualityMetrics(_)) => TelemetryKind::AirQuality,
            Some(telemetry::Variant::PowerMetrics(_)) => TelemetryKind::Power,
            Some(telemetry::Variant::LocalStats(_)) => TelemetryKind::LocalStats,
            Some(telemetry::Variant::HealthMetrics(_)) => TelemetryKind::Health,
            Some(telemetry::Variant::HostMetrics(_)) => TelemetryKind::Host,
            None => TelemetryKind::Unknown,
        }
    }
}

impl Database {
    /// Append a telemetry sample for a node.
    pub fn insert_telemetry(&self, node_num: u32, telemetry: &Telemetry) -> Result<i64> {
        let timestamp = if telemetry.time != 0 {
            telemetry.time as i64
        } else {
            now_unix()
        };
        let kind = TelemetryKind::of(telemetry);
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO telemetry (node_num, kind, timestamp, telemetry_blob)
                 VALUES (?1, ?2, ?3, ?4)",
                params![node_num, kind.as_str(), timestamp, encode(telemetry)],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Most recent telemetry samples for a node, newest first.
    pub fn recent_telemetry(&self, node_num: u32, limit: u32) -> Result<Vec<Telemetry>> {
        let limit = limit.clamp(1, 1_000) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT telemetry_blob FROM telemetry
                  WHERE node_num = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![node_num, limit], |row| row.get::<_, Vec<u8>>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(decode::<Telemetry>(&row?)?);
            }
            Ok(out)
        })
    }

    /// Append a position fix for a node.
    pub fn insert_position(&self, node_num: u32, position: &Position) -> Result<i64> {
        let timestamp = if position.time != 0 {
            position.time as i64
        } else {
            now_unix()
        };
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO positions
                     (node_num, timestamp, latitude_i, longitude_i, altitude, position_blob)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    node_num,
                    timestamp,
                    position.latitude_i,
                    position.longitude_i,
                    position.altitude,
                    encode(position),
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    /// Position history for a node, newest first.
    pub fn recent_positions(&self, node_num: u32, limit: u32) -> Result<Vec<Position>> {
        let limit = limit.clamp(1, 5_000) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT position_blob FROM positions
                  WHERE node_num = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![node_num, limit], |row| row.get::<_, Vec<u8>>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(decode::<Position>(&row?)?);
            }
            Ok(out)
        })
    }

    /// The most recent position fix for a node, if any.
    pub fn latest_position(&self, node_num: u32) -> Result<Option<Position>> {
        let mut positions = self.recent_positions(node_num, 1)?;
        Ok(positions.pop())
    }

    /// Delete telemetry and positions last recorded before `timestamp`.
    /// Returns the number of rows removed.
    pub fn prune_before(&self, timestamp: i64) -> Result<usize> {
        self.with_conn(|conn| {
            let a = conn.execute(
                "DELETE FROM telemetry WHERE timestamp < ?1",
                params![timestamp],
            )?;
            let b = conn.execute(
                "DELETE FROM positions WHERE timestamp < ?1",
                params![timestamp],
            )?;
            Ok(a + b)
        })
    }

    /// Row counts as `(telemetry, positions)`.
    pub fn telemetry_counts(&self) -> Result<(u32, u32)> {
        self.with_conn(|conn| {
            let t: u32 = conn.query_row("SELECT COUNT(*) FROM telemetry", [], |row| row.get(0))?;
            let p: u32 = conn.query_row("SELECT COUNT(*) FROM positions", [], |row| row.get(0))?;
            Ok((t, p))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshtastic_protobufs::meshtastic::{DeviceMetrics, EnvironmentMetrics};

    fn device_metrics_telemetry(battery: u32, time: u32) -> Telemetry {
        Telemetry {
            time,
            variant: Some(telemetry::Variant::DeviceMetrics(DeviceMetrics {
                battery_level: Some(battery),
                ..Default::default()
            })),
        }
    }

    #[test]
    fn telemetry_appends_and_reads_back() {
        let db = Database::open_in_memory().unwrap();
        db.insert_telemetry(7, &device_metrics_telemetry(80, 1000))
            .unwrap();
        db.insert_telemetry(7, &device_metrics_telemetry(70, 2000))
            .unwrap();
        let recent = db.recent_telemetry(7, 10).unwrap();
        assert_eq!(recent.len(), 2);
        // Newest first.
        assert_eq!(recent[0].time, 2000);
        assert_eq!(TelemetryKind::of(&recent[0]), TelemetryKind::Device);

        let env = Telemetry {
            time: 3000,
            variant: Some(telemetry::Variant::EnvironmentMetrics(EnvironmentMetrics {
                temperature: Some(21.0),
                ..Default::default()
            })),
        };
        db.insert_telemetry(7, &env).unwrap();
        assert_eq!(
            TelemetryKind::of(&db.recent_telemetry(7, 1).unwrap()[0]),
            TelemetryKind::Environment
        );
    }

    #[test]
    fn position_history_and_pruning() {
        let db = Database::open_in_memory().unwrap();
        for i in 0..5u32 {
            db.insert_position(
                9,
                &Position {
                    latitude_i: Some(40_000_000 + i as i32),
                    longitude_i: Some(-105_000_000),
                    time: 1000 + i,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        assert_eq!(db.recent_positions(9, 2).unwrap().len(), 2);
        assert_eq!(db.latest_position(9).unwrap().unwrap().time, 1004);
        assert_eq!(db.telemetry_counts().unwrap(), (0, 5));

        // Remove everything strictly before 1004.
        assert_eq!(db.prune_before(1004).unwrap(), 4);
        assert_eq!(db.telemetry_counts().unwrap(), (0, 1));
    }
}
