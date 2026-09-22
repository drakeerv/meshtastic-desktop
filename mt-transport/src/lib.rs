//! # mt-transport
//!
//! Transport layer connecting to Meshtastic nodes over BLE, USB serial and
//! TCP (WiFi), plus a fully simulated in-process node for development and a
//! unified discovery facility (BLE scan, serial port enumeration, mDNS).
//!
//! The crate exposes a single, transport agnostic surface:
//!
//! - [`spawn_transport`] connects to a [`DeviceAddress`] and returns a
//!   [`TransportHandle`] for sending `ToRadio` messages plus an event
//!   stream carrying `FromRadio` messages, connection state and device
//!   logs.
//! - [`spawn_discovery`] runs background discovery and reports device
//!   list snapshots.
//!
//! Reconnection policy, handshakes and heartbeats are deliberately *not*
//! implemented here; they belong to `mt-core`'s supervisor.

pub mod ble;
mod ble_pair;
pub mod discovery;
pub mod mock;
pub mod serial;
mod stream_io;
pub mod tcp;

pub use discovery::{
    DiscoveredDevice, DiscoveryCommand, DiscoveryEvent, DiscoveryHandle, spawn_discovery,
};

use std::fmt;
use std::str::FromStr;

use meshtastic_protobufs::meshtastic::{FromRadio, ToRadio};
use tokio::sync::mpsc;

/// The kind of link a [`DeviceAddress`] points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    Ble,
    Serial,
    Tcp,
    Mock,
}

impl TransportKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransportKind::Ble => "BLE",
            TransportKind::Serial => "Serial",
            TransportKind::Tcp => "WiFi",
            TransportKind::Mock => "Mock",
        }
    }
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Address of a node, using the same prefix scheme as the official
/// Android app: `x` = BLE, `t` = TCP, `s` = serial, `m` = mock.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DeviceAddress {
    /// Bluetooth MAC address, e.g. `aa:bb:cc:dd:ee:ff`.
    Ble(String),
    /// Serial port path, e.g. `/dev/ttyUSB0`.
    Serial(String),
    /// TCP host and port (Meshtastic default port 4403).
    Tcp { host: String, port: u16 },
    /// In-process simulated node name.
    Mock(String),
}

impl DeviceAddress {
    pub fn kind(&self) -> TransportKind {
        match self {
            DeviceAddress::Ble(_) => TransportKind::Ble,
            DeviceAddress::Serial(_) => TransportKind::Serial,
            DeviceAddress::Tcp { .. } => TransportKind::Tcp,
            DeviceAddress::Mock(_) => TransportKind::Mock,
        }
    }

    /// BLE address from a MAC string (any case, `:` separators).
    pub fn ble(mac: impl Into<String>) -> Self {
        DeviceAddress::Ble(mac.into())
    }

    /// Serial address from a port path.
    pub fn serial(path: impl Into<String>) -> Self {
        DeviceAddress::Serial(path.into())
    }

    /// TCP address, applying the default Meshtastic port if needed.
    pub fn tcp(host: impl Into<String>, port: u16) -> Self {
        DeviceAddress::Tcp {
            host: host.into(),
            port,
        }
    }

    /// Mock address.
    pub fn mock(name: impl Into<String>) -> Self {
        DeviceAddress::Mock(name.into())
    }

    /// Short human readable label (without kind).
    pub fn label(&self) -> String {
        match self {
            DeviceAddress::Ble(mac) => mac.to_string(),
            DeviceAddress::Serial(path) => path.clone(),
            DeviceAddress::Tcp { host, port } => format!("{host}:{port}"),
            DeviceAddress::Mock(name) => name.clone(),
        }
    }
}

impl fmt::Display for DeviceAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceAddress::Ble(mac) => write!(f, "x{mac}"),
            DeviceAddress::Serial(path) => write!(f, "s{path}"),
            DeviceAddress::Tcp { host, port } => write!(f, "t{host}:{port}"),
            DeviceAddress::Mock(name) => write!(f, "m{name}"),
        }
    }
}

impl FromStr for DeviceAddress {
    type Err = TransportError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || TransportError::InvalidAddress(s.to_string());
        let (kind, rest) = s.split_at(1.min(s.len()));
        match kind {
            "x" => Ok(DeviceAddress::Ble(rest.to_string())),
            "s" => Ok(DeviceAddress::Serial(rest.to_string())),
            "t" => {
                // host or host:port
                if let Some((host, port)) = rest.rsplit_once(':') {
                    let port = port.parse().map_err(|_| err())?;
                    Ok(DeviceAddress::Tcp {
                        host: host.to_string(),
                        port,
                    })
                } else {
                    Ok(DeviceAddress::Tcp {
                        host: rest.to_string(),
                        port: mt_protocol::constants::TCP_PORT,
                    })
                }
            }
            "m" => Ok(DeviceAddress::Mock(rest.to_string())),
            _ => Err(err()),
        }
    }
}

impl DeviceAddress {
    /// Parse a user-typed address, inferring the transport from its shape.
    ///
    /// Unlike [`FromStr`](std::str::FromStr), which requires a single-letter
    /// prefix, this accepts the forms the Connect screen advertises: a bare
    /// BLE MAC (`aa:bb:cc:dd:ee:ff`), a serial path (`/dev/ttyACM0`, `COM3`),
    /// or a host/IP with an optional port. Explicit prefixes still work, and
    /// win when the remainder clearly matches that transport.
    pub fn parse_manual(input: &str) -> Result<Self, TransportError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(TransportError::InvalidAddress(input.to_string()));
        }

        // Explicit prefixes first, but only when the remainder clearly
        // matches, so a hostname like `test.local` is not read as `t` + host.
        if let Some(rest) = input.strip_prefix('x') {
            if is_mac(rest) {
                return Ok(DeviceAddress::Ble(normalize_mac(rest)));
            }
        }
        if let Some(rest) = input.strip_prefix('s') {
            if is_serial_path(rest) {
                return Ok(DeviceAddress::Serial(rest.to_string()));
            }
        }
        if let Some(rest) = input.strip_prefix('t') {
            if is_strong_host(rest) {
                return Ok(tcp_from(rest));
            }
        }

        // Unprefixed, inferred from shape.
        if is_mac(input) {
            return Ok(DeviceAddress::Ble(normalize_mac(input)));
        }
        if is_serial_path(input) {
            return Ok(DeviceAddress::Serial(input.to_string()));
        }
        if is_loose_host(input) {
            return Ok(tcp_from(input));
        }

        // Last resort: the strict prefix parser (e.g. an explicit mock name).
        input.parse()
    }
}

/// Whether `s` is a six-octet MAC address separated by `:` or `-`.
fn is_mac(s: &str) -> bool {
    let mut groups = 0;
    for part in s.split([':', '-']) {
        if part.len() != 2 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
        groups += 1;
    }
    groups == 6
}

/// Normalise a MAC to the colon-separated form the BLE stack expects.
fn normalize_mac(s: &str) -> String {
    s.replace('-', ":").to_ascii_lowercase()
}

/// Whether `s` is a serial port path (`/dev/...`) or a DOS name (`COM3`).
fn is_serial_path(s: &str) -> bool {
    if s.starts_with('/') {
        return true;
    }
    let (prefix, rest) = s.split_at(3.min(s.len()));
    prefix.eq_ignore_ascii_case("COM")
        && !rest.is_empty()
        && rest.chars().all(|c| c.is_ascii_digit())
}

/// Whether `s` is an IP literal, `localhost`, or `host:port`.
fn is_strong_host(s: &str) -> bool {
    if s.eq_ignore_ascii_case("localhost") || is_ipv4(s) {
        return true;
    }
    s.rsplit_once(':')
        .map(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok())
        .unwrap_or(false)
}

/// Whether `s` is a strong host or a dotted host name (`radio.local`).
fn is_loose_host(s: &str) -> bool {
    is_strong_host(s) || (s.contains('.') && !s.chars().any(char::is_whitespace) && s.len() > 1)
}

/// Whether `s` is a dotted-decimal IPv4 literal.
fn is_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
}

/// Build a TCP address, applying the default Meshtastic port when omitted.
fn tcp_from(s: &str) -> DeviceAddress {
    if let Some((host, port)) = s.rsplit_once(':') {
        if let Ok(port) = port.parse::<u16>() {
            if !host.is_empty() {
                return DeviceAddress::Tcp {
                    host: host.to_string(),
                    port,
                };
            }
        }
    }
    DeviceAddress::Tcp {
        host: s.to_string(),
        port: mt_protocol::constants::TCP_PORT,
    }
}

/// Everything that can go wrong below the protocol layer.
///
/// Error payloads are plain strings (not source error values) so the type
/// stays `Clone`, which the event pipeline requires.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportError {
    #[error("connection closed")]
    Closed,
    #[error("i/o error: {0}")]
    Io(String),
    #[error("bluetooth error: {0}")]
    Ble(String),
    #[error("serial error: {0}")]
    Serial(String),
    #[error("operation timed out")]
    Timeout,
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    #[error("device not found: {0}")]
    NotFound(String),
    #[error("transport is shutting down")]
    Shutdown,
}

impl From<std::io::Error> for TransportError {
    fn from(e: std::io::Error) -> Self {
        TransportError::Io(e.to_string())
    }
}

impl From<btleplug::Error> for TransportError {
    fn from(e: btleplug::Error) -> Self {
        TransportError::Ble(e.to_string())
    }
}

/// Events produced by a running transport.
#[derive(Debug, Clone)]
pub enum TransportEvent {
    /// The link is established; safe to start the handshake.
    Connected,
    /// The link is down. Emitted exactly once per transport lifetime as the
    /// final event; carry an error when the cause is abnormal.
    Disconnected { error: Option<TransportError> },
    /// A decoded `FromRadio` message from the node.
    ///
    /// Boxed to keep `TransportEvent` small: the protobuf dwarfs every other
    /// variant and these events travel through bounded channels.
    FromRadio(Box<FromRadio>),
    /// A line of device debug output (serial junk / BLE LOGRADIO).
    DeviceLog(String),
    /// BLE pairing needs the passkey shown on the device's screen.
    BlePairingRequest { address: String },
}

/// Commands accepted by a running transport.
#[derive(Debug)]
pub(crate) enum TransportCommand {
    Send(ToRadio),
    /// The passkey the user read off the device, for BLE pairing.
    BlePasskey(u32),
    Disconnect,
}

/// Cloneable handle for talking to a running transport.
#[derive(Debug, Clone)]
pub struct TransportHandle {
    cmd: mpsc::Sender<TransportCommand>,
}

impl TransportHandle {
    /// Queue a `ToRadio` message for transmission.
    pub async fn send(&self, msg: ToRadio) -> Result<(), TransportError> {
        self.cmd
            .send(TransportCommand::Send(msg))
            .await
            .map_err(|_| TransportError::Closed)
    }

    /// Supply the BLE pairing passkey the user read off the device.
    pub async fn submit_ble_passkey(&self, passkey: u32) {
        let _ = self.cmd.send(TransportCommand::BlePasskey(passkey)).await;
    }

    /// Ask the transport to close the link gracefully.
    pub async fn disconnect(&self) {
        let _ = self.cmd.send(TransportCommand::Disconnect).await;
    }

    /// Whether the transport task has exited.
    pub fn is_closed(&self) -> bool {
        self.cmd.is_closed()
    }
}

/// Spawn a transport for `address`.
///
/// The returned receiver yields [`TransportEvent`]s and always terminates
/// with a `Disconnected` event, allowing the supervisor in `mt-core` to
/// implement its own reconnect policy.
///
/// # Panics
///
/// Must be called from within a tokio runtime.
pub fn spawn_transport(
    address: DeviceAddress,
) -> (TransportHandle, mpsc::Receiver<TransportEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (evt_tx, evt_rx) = mpsc::channel(256);

    tokio::spawn(async move {
        let result = match address.clone() {
            DeviceAddress::Ble(mac) => ble::run(&mac, cmd_rx, evt_tx.clone()).await,
            DeviceAddress::Serial(path) => serial::run(&path, cmd_rx, evt_tx.clone()).await,
            DeviceAddress::Tcp { host, port } => {
                tcp::run(&host, port, cmd_rx, evt_tx.clone()).await
            }
            DeviceAddress::Mock(name) => mock::run(&name, cmd_rx, evt_tx.clone()).await,
        };
        match result {
            Ok(()) => {
                let _ = evt_tx
                    .send(TransportEvent::Disconnected { error: None })
                    .await;
            }
            Err(error) => {
                tracing::warn!(%address, %error, "transport ended with error");
                let _ = evt_tx
                    .send(TransportEvent::Disconnected { error: Some(error) })
                    .await;
            }
        }
    });

    (TransportHandle { cmd: cmd_tx }, evt_rx)
}

/// Accumulates junk bytes into log lines so the UI can show device debug
/// output without spamming per-byte events.
#[derive(Debug, Default)]
pub(crate) struct JunkLogger {
    line: Vec<u8>,
}

impl JunkLogger {
    pub fn push(&mut self, byte: u8, evt_tx: &mpsc::Sender<TransportEvent>) {
        if byte == b'\n' {
            self.flush(evt_tx);
        } else {
            self.line.push(byte);
            if self.line.len() > 1024 {
                self.flush(evt_tx);
            }
        }
    }

    pub fn flush(&mut self, evt_tx: &mpsc::Sender<TransportEvent>) {
        if self.line.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(&self.line).trim().to_string();
        self.line.clear();
        if !text.is_empty() {
            let _ = evt_tx.try_send(TransportEvent::DeviceLog(text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_round_trip() {
        for addr in [
            DeviceAddress::Ble("aa:bb:cc:dd:ee:ff".into()),
            DeviceAddress::Serial("/dev/ttyUSB0".into()),
            DeviceAddress::Tcp {
                host: "192.168.1.42".into(),
                port: 4403,
            },
            DeviceAddress::Mock("demo".into()),
        ] {
            let s = addr.to_string();
            let back: DeviceAddress = s.parse().unwrap();
            assert_eq!(addr, back);
        }
    }

    #[test]
    fn tcp_address_defaults_port() {
        let addr: DeviceAddress = "tmeshtastic.local".parse().unwrap();
        assert_eq!(
            addr,
            DeviceAddress::Tcp {
                host: "meshtastic.local".into(),
                port: 4403
            }
        );
        let addr: DeviceAddress = "t10.0.0.7:5000".parse().unwrap();
        assert_eq!(
            addr,
            DeviceAddress::Tcp {
                host: "10.0.0.7".into(),
                port: 5000
            }
        );
    }

    #[test]
    fn bad_addresses_rejected() {
        assert!("nonsense".parse::<DeviceAddress>().is_err());
        assert!("".parse::<DeviceAddress>().is_err());
    }

    #[test]
    fn ipv6_style_addresses_keep_last_colon_split() {
        // rsplit_once grabs the last colon: port must be numeric to parse.
        let addr: DeviceAddress = "tfe80::1:4403".parse().unwrap();
        assert_eq!(
            addr,
            DeviceAddress::Tcp {
                host: "fe80::1".into(),
                port: 4403
            }
        );
    }

    #[test]
    fn manual_parses_bare_ble_mac() {
        let addr = DeviceAddress::parse_manual("10:BD:A3:5B:07:F9").unwrap();
        assert_eq!(addr, DeviceAddress::Ble("10:bd:a3:5b:07:f9".into()));
        // The explicit prefix still works, as does dash-separated input.
        assert_eq!(
            DeviceAddress::parse_manual("x10:bd:a3:5b:07:f9").unwrap(),
            DeviceAddress::Ble("10:bd:a3:5b:07:f9".into())
        );
        assert_eq!(
            DeviceAddress::parse_manual("10-BD-A3-5B-07-F9").unwrap(),
            DeviceAddress::Ble("10:bd:a3:5b:07:f9".into())
        );
    }

    #[test]
    fn manual_parses_serial_paths() {
        assert_eq!(
            DeviceAddress::parse_manual("/dev/ttyACM0").unwrap(),
            DeviceAddress::Serial("/dev/ttyACM0".into())
        );
        assert_eq!(
            DeviceAddress::parse_manual("s/dev/ttyUSB0").unwrap(),
            DeviceAddress::Serial("/dev/ttyUSB0".into())
        );
        assert_eq!(
            DeviceAddress::parse_manual("COM3").unwrap(),
            DeviceAddress::Serial("COM3".into())
        );
    }

    #[test]
    fn manual_parses_hosts_and_ports() {
        assert_eq!(
            DeviceAddress::parse_manual("192.168.1.42").unwrap(),
            DeviceAddress::Tcp {
                host: "192.168.1.42".into(),
                port: 4403
            }
        );
        assert_eq!(
            DeviceAddress::parse_manual("192.168.1.42:5555").unwrap(),
            DeviceAddress::Tcp {
                host: "192.168.1.42".into(),
                port: 5555
            }
        );
        assert_eq!(
            DeviceAddress::parse_manual("t192.168.1.42").unwrap(),
            DeviceAddress::Tcp {
                host: "192.168.1.42".into(),
                port: 4403
            }
        );
    }

    #[test]
    fn manual_hostnames_starting_with_a_prefix_letter_are_not_misread() {
        // `test.local` must not become `t` + `est.local`, and `meshtastic`
        // must not become a mock address.
        assert_eq!(
            DeviceAddress::parse_manual("test.local").unwrap(),
            DeviceAddress::Tcp {
                host: "test.local".into(),
                port: 4403
            }
        );
        assert_eq!(
            DeviceAddress::parse_manual("meshtastic.local").unwrap(),
            DeviceAddress::Tcp {
                host: "meshtastic.local".into(),
                port: 4403
            }
        );
    }

    #[test]
    fn manual_rejects_garbage() {
        assert!(DeviceAddress::parse_manual("   ").is_err());
        assert!(DeviceAddress::parse_manual("not an address").is_err());
    }
}
