//! Well-known constants of the Meshtastic protocol.
//!
//! Values cross-checked against the official clients
//! (`Meshtastic-Android`, `Meshtastic-Apple`, `Meshtastic-JS/web`).

use uuid::Uuid;

/// First byte of a stream frame start marker.
pub const START1: u8 = 0x94;
/// Second byte of a stream frame start marker.
pub const START2: u8 = 0xC3;

/// Wake-up sequence written to serial ports before the first frame so that
/// ESP32 based devices exit their light sleep.
pub const WAKE_BYTES: [u8; 4] = [0x94, 0x94, 0x94, 0x94];

/// Hard limit for a single frame payload, matching the firmware's
/// `MAX_TO_FROM_RADIO_SIZE`.
pub const MAX_FRAME_PAYLOAD: usize = 512;

/// Default TCP port used by WiFi enabled nodes.
pub const TCP_PORT: u16 = 4403;

/// mDNS service advertised by networked nodes.
pub const MDNS_SERVICE_TYPE: &str = "_meshtastic._tcp.local.";

/// Default serial baud rate (8N1).
pub const SERIAL_BAUD: u32 = 115_200;

/// Stage 1 nonce: streams config, channels and the file manifest but skips
/// the node database (`SPECIAL_NONCE_ONLY_CONFIG`).
pub const HANDSHAKE_NONCE_1: u32 = 69_420;
/// Stage 2 nonce: streams the device's stored node database
/// (`SPECIAL_NONCE_ONLY_NODES`).
pub const HANDSHAKE_NONCE_2: u32 = 69_421;

/// The all-nodes broadcast address (`^all`).
pub const BROADCAST_ADDR: u32 = 0xFFFF_FFFF;
/// Node number of the local node before it joins a mesh.
pub const NODENUM_BROADCAST: u32 = 0xFFFF_FFFF;

/// Channel index used for PKI encrypted direct messages.
pub const PKI_CHANNEL_INDEX: u8 = 8;

/// Maximum hop limit proposed for client originated packets.
pub const DEFAULT_HOP_LIMIT: u32 = 3;

/// Meshtastic BLE GATT service.
pub const BLE_SERVICE_UUID: Uuid = Uuid::from_u128(0x6ba1b218_15a8_461f_9fa8_5dcae273eafd);
/// Characteristic the client writes `ToRadio` protobufs to.
pub const BLE_TORADIO_UUID: Uuid = Uuid::from_u128(0xf75c76d2_129e_4dad_a1dd_7866124401e7);
/// Characteristic the client polls for `FromRadio` protobufs.
pub const BLE_FROMRADIO_UUID: Uuid = Uuid::from_u128(0x2c55e69e_4993_11ed_b878_0242ac120002);
/// Characteristic that notifies whenever `FROMRADIO` has data available.
pub const BLE_FROMNUM_UUID: Uuid = Uuid::from_u128(0xed9da18c_a800_4f66_a670_aa7547e34453);
/// Optional characteristic streaming device logs.
pub const BLE_LOGRADIO_UUID: Uuid = Uuid::from_u128(0x5a3d6e49_06e6_4423_9944_e9de8cdf9547);

/// Matches device names like `Meshtastic ab12` or `Talkie A1B2`.
/// The captured hex group is the last four characters of the node number.
pub const BLE_NAME_SUFFIX_LEN: usize = 4;

/// Extract the 4 hex digit suffix from a BLE advertisement name, if any.
/// Mirrors the official apps' `^.*_([0-9a-fA-F]{4})$` pattern, but is
/// deliberately lenient with separators so that `Meshtastic 9f2c`,
/// `T-Beam 4021` and `m_4021` all work.
pub fn ble_name_suffix(name: &str) -> Option<u32> {
    let trimmed = name.trim_end();
    if trimmed.len() < BLE_NAME_SUFFIX_LEN {
        return None;
    }
    let tail = &trimmed[trimmed.len() - BLE_NAME_SUFFIX_LEN..];
    if !tail.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    // Ensure there is a non-hex boundary before the suffix so we do not
    // grab the tail of a longer hex run (e.g. an entirely hex MAC string).
    let boundary = trimmed
        .get(..trimmed.len() - BLE_NAME_SUFFIX_LEN)
        .and_then(|p| p.chars().last())
        .map(|c| !c.is_ascii_hexdigit())
        .unwrap_or(true);
    if !boundary {
        return None;
    }
    u32::from_str_radix(tail, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_suffix() {
        assert_eq!(ble_name_suffix("Meshtastic ab12"), Some(0xAB12));
        assert_eq!(ble_name_suffix("Talkie 9F2C"), Some(0x9F2C));
        assert_eq!(ble_name_suffix("T1000_E5C6"), Some(0xE5C6));
    }

    #[test]
    fn rejects_non_names() {
        assert_eq!(ble_name_suffix("Meshtastic"), None);
        assert_eq!(ble_name_suffix("JBL Flip 6"), None);
        // Whole string hex: this is likely a MAC, not a Meshtastic name.
        assert_eq!(ble_name_suffix("abcdef"), None);
    }
}
