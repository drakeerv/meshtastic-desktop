//! Device and module configuration storage.
//!
//! Each `Config` / `ModuleConfig` variant is keyed by its section name, so
//! a repeated config download updates sections in place and a genuinely
//! missing section can be detected by the UI.

use meshtastic_protobufs::meshtastic::{Config, ModuleConfig};
use rusqlite::params;

use crate::schema::{config_section, decode, encode, module_config_section};
use crate::{Database, Result, now_unix};

impl Database {
    /// Insert or update every given device config section.
    pub fn replace_configs(&self, configs: &[Config]) -> Result<()> {
        let now = now_unix();
        self.with_tx(|tx| {
            for config in configs {
                tx.execute(
                    "INSERT INTO configs (section, config_blob, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(section) DO UPDATE SET
                         config_blob = excluded.config_blob,
                         updated_at  = excluded.updated_at",
                    params![config_section(config), encode(config), now],
                )?;
            }
            Ok(())
        })
    }

    /// All stored device config sections.
    pub fn list_configs(&self) -> Result<Vec<Config>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT config_blob FROM configs ORDER BY section ASC")?;
            let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(decode::<Config>(&row?)?);
            }
            Ok(out)
        })
    }

    /// Insert or update every given module config section.
    pub fn replace_module_configs(&self, configs: &[ModuleConfig]) -> Result<()> {
        let now = now_unix();
        self.with_tx(|tx| {
            for config in configs {
                tx.execute(
                    "INSERT INTO module_configs (section, config_blob, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(section) DO UPDATE SET
                         config_blob = excluded.config_blob,
                         updated_at  = excluded.updated_at",
                    params![module_config_section(config), encode(config), now],
                )?;
            }
            Ok(())
        })
    }

    /// All stored module config sections.
    pub fn list_module_configs(&self) -> Result<Vec<ModuleConfig>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT config_blob FROM module_configs ORDER BY section ASC")?;
            let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(decode::<ModuleConfig>(&row?)?);
            }
            Ok(out)
        })
    }

    /// Drop all cached configuration (e.g. before a fresh download).
    pub fn clear_configs(&self) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute_batch("DELETE FROM configs; DELETE FROM module_configs;")?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshtastic_protobufs::meshtastic::{config, module_config};

    #[test]
    fn configs_upsert_by_section() {
        let db = Database::open_in_memory().unwrap();
        let lora = Config {
            payload_variant: Some(config::PayloadVariant::Lora(config::LoRaConfig {
                region: config::lo_ra_config::RegionCode::Us as i32,
                hop_limit: 3,
                ..Default::default()
            })),
        };
        let device = Config {
            payload_variant: Some(config::PayloadVariant::Device(
                config::DeviceConfig::default(),
            )),
        };
        db.replace_configs(&[lora, device]).unwrap();
        assert_eq!(db.list_configs().unwrap().len(), 2);

        // Update one section: still two rows, new value present.
        let lora2 = Config {
            payload_variant: Some(config::PayloadVariant::Lora(config::LoRaConfig {
                region: config::lo_ra_config::RegionCode::Eu868 as i32,
                ..Default::default()
            })),
        };
        db.replace_configs(&[lora2]).unwrap();
        let configs = db.list_configs().unwrap();
        assert_eq!(configs.len(), 2);
        let found = configs.iter().find_map(|c| match &c.payload_variant {
            Some(config::PayloadVariant::Lora(l)) => Some(l.region),
            _ => None,
        });
        assert_eq!(found, Some(config::lo_ra_config::RegionCode::Eu868 as i32));
    }

    #[test]
    fn module_configs_round_trip() {
        let db = Database::open_in_memory().unwrap();
        db.replace_module_configs(&[ModuleConfig {
            payload_variant: Some(module_config::PayloadVariant::Mqtt(
                module_config::MqttConfig::default(),
            )),
        }])
        .unwrap();
        assert_eq!(db.list_module_configs().unwrap().len(), 1);
        db.clear_configs().unwrap();
        assert!(db.list_configs().unwrap().is_empty());
        assert!(db.list_module_configs().unwrap().is_empty());
    }
}
