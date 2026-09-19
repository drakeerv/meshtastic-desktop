//! Small key/value table for per-device metadata.

use rusqlite::{OptionalExtension, params};

use crate::{Database, Result, now_unix};

/// Well-known metadata keys.
pub mod keys {
    pub const FIRMWARE_VERSION: &str = "firmware_version";
    pub const DEVICE_STATE_VERSION: &str = "device_state_version";
    pub const MY_NODE_NUM: &str = "my_node_num";
    pub const REBOOT_COUNT: &str = "reboot_count";
    pub const REGION: &str = "region";
    pub const HW_MODEL: &str = "hw_model";
    pub const LAST_CONNECTED: &str = "last_connected";
}

impl Database {
    /// Store a metadata value, overwriting any previous one.
    pub fn put_meta(&self, key: &str, value: &str) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO meta (key, value, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET
                     value = excluded.value,
                     updated_at = excluded.updated_at",
                params![key, value, now_unix()],
            )?;
            Ok(())
        })
    }

    /// Read a metadata value.
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT value FROM meta WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?)
        })
    }

    /// Read a metadata value parsed as a `u32`.
    pub fn get_meta_u32(&self, key: &str) -> Result<Option<u32>> {
        Ok(self.get_meta(key)?.and_then(|v| v.parse().ok()))
    }

    /// Remove a metadata value.
    pub fn remove_meta(&self, key: &str) -> Result<usize> {
        self.with_conn(|conn| Ok(conn.execute("DELETE FROM meta WHERE key = ?1", params![key])?))
    }

    /// Convenience: recorded firmware version.
    pub fn firmware_version(&self) -> Result<Option<String>> {
        self.get_meta(keys::FIRMWARE_VERSION)
    }

    /// Convenience: store the firmware version.
    pub fn set_firmware_version(&self, version: &str) -> Result<()> {
        self.put_meta(keys::FIRMWARE_VERSION, version)
    }

    /// Convenience: the node number of the connected device.
    pub fn my_node_num(&self) -> Result<Option<u32>> {
        self.get_meta_u32(keys::MY_NODE_NUM)
    }

    /// Convenience: record the connected device's node number.
    pub fn set_my_node_num(&self, num: u32) -> Result<()> {
        self.put_meta(keys::MY_NODE_NUM, &num.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_round_trip_and_overwrite() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.get_meta("x").unwrap(), None);
        db.put_meta("x", "1").unwrap();
        assert_eq!(db.get_meta("x").unwrap().as_deref(), Some("1"));
        db.put_meta("x", "2").unwrap();
        assert_eq!(db.get_meta_u32("x").unwrap(), Some(2));
        assert_eq!(db.remove_meta("x").unwrap(), 1);
        assert_eq!(db.get_meta("x").unwrap(), None);
    }

    #[test]
    fn typed_helpers() {
        let db = Database::open_in_memory().unwrap();
        db.set_firmware_version("2.7.8.b8942c9").unwrap();
        db.set_my_node_num(0x0042_A1B2).unwrap();
        assert_eq!(
            db.firmware_version().unwrap().as_deref(),
            Some("2.7.8.b8942c9")
        );
        assert_eq!(db.my_node_num().unwrap(), Some(0x0042_A1B2));
    }
}
