//! Channel configuration storage.
//!
//! Channels are a fixed eight-slot table on the device; the client keeps
//! the most recent snapshot. A config download replaces the whole set in
//! one transaction so partial updates can never leave a stale slot behind.

use meshtastic_protobufs::meshtastic::Channel;
use rusqlite::{OptionalExtension, params};

use crate::schema::{decode, encode};
use crate::{Database, Result, now_unix};

impl Database {
    /// Replace every stored channel with `channels`.
    pub fn replace_channels(&self, channels: &[Channel]) -> Result<()> {
        let now = now_unix();
        self.with_tx(|tx| {
            tx.execute("DELETE FROM channels", [])?;
            for channel in channels {
                let settings = channel.settings.as_ref();
                tx.execute(
                    "INSERT INTO channels (idx, name, role, psk, channel_blob, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        channel.index,
                        settings.map(|s| s.name.as_str()),
                        channel.role,
                        settings.map(|s| s.psk.as_slice()),
                        encode(channel),
                        now,
                    ],
                )?;
            }
            Ok(())
        })
    }

    /// Insert or update a single channel slot (used while a config
    /// download streams in one channel at a time).
    pub fn upsert_channel(&self, channel: &Channel) -> Result<()> {
        let settings = channel.settings.as_ref();
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO channels (idx, name, role, psk, channel_blob, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(idx) DO UPDATE SET
                     name = excluded.name,
                     role = excluded.role,
                     psk = excluded.psk,
                     channel_blob = excluded.channel_blob,
                     updated_at = excluded.updated_at",
                params![
                    channel.index,
                    settings.map(|s| s.name.as_str()),
                    channel.role,
                    settings.map(|s| s.psk.as_slice()),
                    encode(channel),
                    now_unix(),
                ],
            )?;
            Ok(())
        })
    }

    /// All stored channels ordered by index.
    pub fn list_channels(&self) -> Result<Vec<Channel>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT channel_blob FROM channels ORDER BY idx ASC")?;
            let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
            let mut channels = Vec::new();
            for row in rows {
                channels.push(decode::<Channel>(&row?)?);
            }
            Ok(channels)
        })
    }

    /// Fetch a single channel slot.
    pub fn get_channel(&self, index: u32) -> Result<Option<Channel>> {
        self.with_conn(|conn| {
            let blob = conn
                .query_row(
                    "SELECT channel_blob FROM channels WHERE idx = ?1",
                    params![index],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()?;
            match blob {
                Some(bytes) => Ok(Some(decode(&bytes)?)),
                None => Ok(None),
            }
        })
    }

    /// Number of stored channel slots.
    pub fn channel_count(&self) -> Result<u32> {
        self.with_conn(|conn| {
            Ok(conn.query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshtastic_protobufs::meshtastic::{ChannelSettings, channel};

    fn channel(index: i32, name: &str) -> Channel {
        Channel {
            index,
            role: channel::Role::Primary as i32,
            settings: Some(ChannelSettings {
                name: name.into(),
                psk: vec![1, 2, 3, 4],
                ..Default::default()
            }),
        }
    }

    #[test]
    fn replace_is_atomic_and_ordered() {
        let db = Database::open_in_memory().unwrap();
        db.replace_channels(&[channel(0, "Primary"), channel(1, "Team")])
            .unwrap();
        assert_eq!(db.channel_count().unwrap(), 2);
        let list = db.list_channels().unwrap();
        assert_eq!(list[0].settings.as_ref().unwrap().name, "Primary");
        assert_eq!(list[1].settings.as_ref().unwrap().name, "Team");

        // Replacing with a shorter set drops stale slots.
        db.replace_channels(&[channel(0, "Renamed")]).unwrap();
        assert_eq!(db.channel_count().unwrap(), 1);
        assert_eq!(
            db.get_channel(0).unwrap().unwrap().settings.unwrap().name,
            "Renamed"
        );
    }
}
