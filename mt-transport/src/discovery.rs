//! Device discovery: BLE scanning, serial port enumeration and mDNS
//! browsing, unified into one device list snapshot stream.
//!
//! - Serial ports are polled every couple of seconds (the `serialport`
//!   crate has no hotplug events on Linux).
//! - mDNS (`_meshtastic._tcp.local.`) runs continuously while the
//!   discovery task is alive.
//! - BLE scanning is radio-expensive, so it is started/stopped on command
//!   from the UI.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use btleplug::api::{Central as _, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use tokio::sync::{broadcast, mpsc};

use mt_protocol::constants::{BLE_SERVICE_UUID, MDNS_SERVICE_TYPE};

use crate::{DeviceAddress, TransportError};

/// A device found by discovery, ready to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredDevice {
    pub address: DeviceAddress,
    /// Advertised or port-provided name.
    pub name: String,
    /// One-line detail for the UI (kind, signal, port type...).
    pub detail: String,
    /// BLE signal strength, when known.
    pub rssi: Option<i16>,
}

/// Commands to steer the discovery task.
#[derive(Debug)]
pub enum DiscoveryCommand {
    /// Begin scanning for BLE devices (idempotent).
    StartBleScan,
    /// Stop the BLE scan and drop BLE results.
    StopBleScan,
}

/// Events emitted by the discovery task.
#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    /// Full device list snapshot (the list is small; snapshots keep the UI
    /// trivially consistent).
    DevicesUpdated(Vec<DiscoveredDevice>),
    /// Non-fatal discovery error (e.g. no bluetooth adapter present).
    Error(String),
}

/// Handle for steering discovery and subscribing to its results.
#[derive(Debug, Clone)]
pub struct DiscoveryHandle {
    cmd: mpsc::Sender<DiscoveryCommand>,
    events: broadcast::Sender<DiscoveryEvent>,
}

impl DiscoveryHandle {
    pub async fn start_ble_scan(&self) {
        let _ = self.cmd.send(DiscoveryCommand::StartBleScan).await;
    }

    pub async fn stop_ble_scan(&self) {
        let _ = self.cmd.send(DiscoveryCommand::StopBleScan).await;
    }

    /// Subscribe to device list snapshots and errors.
    pub fn subscribe(&self) -> broadcast::Receiver<DiscoveryEvent> {
        self.events.subscribe()
    }

    /// Non-blocking scan start (for callers that cannot await).
    pub fn try_start_ble_scan(&self) -> bool {
        self.cmd.try_send(DiscoveryCommand::StartBleScan).is_ok()
    }

    /// Non-blocking scan stop.
    pub fn try_stop_ble_scan(&self) -> bool {
        self.cmd.try_send(DiscoveryCommand::StopBleScan).is_ok()
    }
}

/// Spawn the discovery service. Serial + mDNS run immediately; BLE scanning
/// waits for a command.
///
/// # Panics
///
/// Must be called from within a tokio runtime.
pub fn spawn_discovery() -> DiscoveryHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let (evt_tx, _) = broadcast::channel(64);
    tokio::spawn(run(cmd_rx, evt_tx.clone()));
    DiscoveryHandle {
        cmd: cmd_tx,
        events: evt_tx,
    }
}

async fn run(
    mut cmd_rx: mpsc::Receiver<DiscoveryCommand>,
    evt_tx: broadcast::Sender<DiscoveryEvent>,
) {
    let mut state = DeviceState::new();

    // serial ports
    let (serial_tx, mut serial_rx) = mpsc::channel(4);
    tokio::spawn(poll_serial_ports(serial_tx));

    // mDNS
    let (mdns_tx, mut mdns_rx) = mpsc::channel(64);
    if let Err(e) = spawn_mdns(mdns_tx).await {
        let _ = evt_tx.send(DiscoveryEvent::Error(format!("mDNS unavailable: {e}")));
    }

    // BLE
    // (scan_tx, ble_rx): bool = "scanning?" control line; the second
    // channel carries results back.
    let (scan_tx, scan_rx) = mpsc::channel::<bool>(4);
    let (ble_tx, mut ble_rx) = mpsc::channel::<BleUpdate>(64);
    let ble_task = tokio::spawn(ble_scan_loop(scan_rx, ble_tx));

    // Publish an initial snapshot.
    let _ = evt_tx.send(DiscoveryEvent::DevicesUpdated(state.devices()));

    let mut snapshot = tokio::time::interval(Duration::from_millis(500));
    snapshot.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut dirty = true;

    loop {
        tokio::select! {
            biased;

            maybe_cmd = cmd_rx.recv() => {
                match maybe_cmd {
                    None => {
                        ble_task.abort();
                        return;
                    }
                    Some(DiscoveryCommand::StartBleScan) => {
                        state.ble_visible.clear();
                        dirty = true;
                        let _ = scan_tx.send(true).await;
                    }
                    Some(DiscoveryCommand::StopBleScan) => {
                        state.ble_visible.clear();
                        dirty = true;
                        let _ = scan_tx.send(false).await;
                    }
                }
            }

            Some(list) = serial_rx.recv() => {
                if state.set_serial(list) {
                    dirty = true;
                }
            }

            Some(event) = mdns_rx.recv() => {
                if state.apply_mdns(event) {
                    dirty = true;
                }
            }

            Some(update) = ble_rx.recv() => {
                if state.apply_ble(update) {
                    dirty = true;
                }
            }

            _ = snapshot.tick() => {
                if dirty {
                    dirty = false;
                    let _ = evt_tx.send(DiscoveryEvent::DevicesUpdated(state.devices()));
                }
            }
        }
    }
}

// Serial ports

async fn poll_serial_ports(tx: mpsc::Sender<Vec<DiscoveredDevice>>) {
    loop {
        let list = tokio::task::spawn_blocking(|| match serialport::available_ports() {
            Ok(ports) => ports
                .into_iter()
                .filter_map(|p| {
                    // Only USB serial adapters are plausible Meshtastic
                    // radios. Legacy `/dev/ttyS*` (PCI/unknown) ports are
                    // motherboard UARTs and would bury the real devices.
                    let serialport::SerialPortType::UsbPort(info) = &p.port_type else {
                        return None;
                    };
                    let product = info.product.as_deref().filter(|s| !s.is_empty());
                    let manufacturer = info.manufacturer.as_deref().filter(|s| !s.is_empty());
                    let detail = match (manufacturer, product) {
                        (Some(m), Some(pr)) => format!("USB · {m} {pr}"),
                        (Some(m), None) => format!("USB · {m}"),
                        (None, Some(pr)) => format!("USB · {pr}"),
                        (None, None) => "USB serial device".to_string(),
                    };
                    Some(DiscoveredDevice {
                        address: DeviceAddress::serial(p.port_name.clone()),
                        name: p.port_name.clone(),
                        detail,
                        rssi: None,
                    })
                })
                .collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        })
        .await
        .unwrap_or_default();

        if tx.send(list).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

// mDNS

type MdnsUpdate = (String, IpAddr, u16);

async fn spawn_mdns(tx: mpsc::Sender<MdnsUpdate>) -> Result<(), TransportError> {
    let daemon = ServiceDaemon::new().map_err(|e| TransportError::Serial(format!("{e}")))?;
    let receiver = daemon
        .browse(MDNS_SERVICE_TYPE)
        .map_err(|e| TransportError::Serial(format!("{e}")))?;

    // mdns-sd delivers on a std blocking channel; bridge into async land.
    tokio::task::spawn_blocking(move || {
        // Keep the daemon alive for the lifetime of this task.
        let _daemon = daemon;
        while let Ok(event) = receiver.recv() {
            if let ServiceEvent::ServiceResolved(info) = event {
                let name = info
                    .get_properties()
                    .iter()
                    .find(|p| p.key() == "shortname")
                    .map(|p| p.val_str().to_string())
                    .unwrap_or_else(|| info.get_hostname().trim_end_matches(".local.").to_string());
                if let Some(ip) = info.get_addresses().iter().next().map(|a| a.to_ip_addr()) {
                    if tx.blocking_send((name, ip, info.get_port())).is_err() {
                        break;
                    }
                }
            }
        }
    });
    Ok(())
}

// BLE

/// A BLE scan result: (mac, advertised name, rssi).
type BleUpdate = (String, String, Option<i16>);

async fn ble_scan_loop(mut scan_rx: mpsc::Receiver<bool>, tx: mpsc::Sender<BleUpdate>) {
    let adapter = match get_adapter().await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(error = %e, "no bluetooth adapter for discovery");
            // Drain commands so the parent is not blocked, then exit.
            while scan_rx.recv().await.is_some() {}
            return;
        }
    };

    let mut scanning = false;
    let mut last_seen: HashMap<String, std::time::Instant> = HashMap::new();

    loop {
        // Poll the control line; while idle just wait for a scan request.
        while !scanning {
            match scan_rx.recv().await {
                Some(true) => {
                    // Scan unfiltered and match in software. BlueZ's
                    // service-UUID discovery filter is unreliable: btleplug
                    // sees only a handful of cached peripherals and misses
                    // otherwise-visible nodes that advertise the mesh
                    // service. The official clients scan broadly too.
                    match adapter.start_scan(ScanFilter::default()).await {
                        Ok(()) => {
                            tracing::debug!("ble discovery scan started");
                            scanning = true;
                        }
                        Err(e) => {
                            // The scan may already be running (BlueZ returns
                            // InProgress); treat that as on.
                            tracing::debug!(error = %e, "ble scan start rejected; assuming active");
                            scanning = true;
                        }
                    }
                }
                Some(false) => {}
                None => return,
            }
        }

        tokio::select! {
            maybe_scan = scan_rx.recv() => {
                match maybe_scan {
                    Some(true) => {}
                    Some(false) | None => {
                        adapter.stop_scan().await.ok();
                        scanning = false;
                        last_seen.clear();
                        if maybe_scan.is_none() {
                            return;
                        }
                        continue;
                    }
                }
            }

            _ = tokio::time::sleep(Duration::from_millis(800)) => {
                // Sweep advertised peripherals.
                match adapter.peripherals().await {
                    Ok(peripherals) => {
                        for p in peripherals {
                            let Ok(Some(props)) = p.properties().await else {
                                continue;
                            };
                            // Require the advertised mesh service. The
                            // four-hex-digit name suffix alone is too loose
                            // (many unrelated devices end that way), so it is
                            // only used to derive the node number, not to
                            // include a device. "meshtastic" in the name is
                            // kept as a fallback for renamed nodes.
                            let service_match = props.services.contains(&BLE_SERVICE_UUID);
                            let name_match = props
                                .local_name
                                .as_deref()
                                .map(|name| name.to_lowercase().contains("meshtastic"))
                                .unwrap_or(false);
                            if !service_match && !name_match {
                                continue;
                            }
                            let name = props
                                .local_name
                                .clone()
                                .unwrap_or_else(|| format!("Meshtastic {}", p.address()));
                            let mac = p.address().to_string();
                            last_seen.insert(mac.clone(), std::time::Instant::now());
                            let _ = tx.try_send((mac, name, props.rssi));
                        }
                        // Drop devices not seen recently (out of range).
                        last_seen.retain(|_, seen| seen.elapsed() < Duration::from_secs(8));
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "ble peripheral sweep failed");
                    }
                }
            }
        }
    }
}

async fn get_adapter() -> Result<Adapter, TransportError> {
    let manager = Manager::new().await?;
    let mut adapters = manager.adapters().await?;
    adapters
        .drain(..)
        .next()
        .ok_or_else(|| TransportError::NotFound("no bluetooth adapter".into()))
}

// Aggregated state

struct DeviceState {
    serial: Vec<DiscoveredDevice>,
    mdns: HashMap<String, DiscoveredDevice>,
    ble_visible: HashMap<String, DiscoveredDevice>,
}

impl DeviceState {
    fn new() -> Self {
        Self {
            serial: Vec::new(),
            mdns: HashMap::new(),
            ble_visible: HashMap::new(),
        }
    }

    fn set_serial(&mut self, list: Vec<DiscoveredDevice>) -> bool {
        if list == self.serial {
            false
        } else {
            self.serial = list;
            true
        }
    }

    fn apply_mdns(&mut self, (name, ip, port): MdnsUpdate) -> bool {
        let key = format!("{ip}:{port}");
        let device = DiscoveredDevice {
            address: DeviceAddress::Tcp {
                host: ip.to_string(),
                port,
            },
            name,
            detail: format!("WiFi {ip}:{port}"),
            rssi: None,
        };
        self.mdns.insert(key, device).is_none()
    }

    fn apply_ble(&mut self, (mac, name, rssi): BleUpdate) -> bool {
        let device = DiscoveredDevice {
            address: DeviceAddress::ble(mac.clone()),
            name: name.clone(),
            detail: "Bluetooth".into(),
            rssi,
        };
        // Report changed rssi/name too, not just first sightings.
        match self.ble_visible.insert(mac, device) {
            None => true,
            Some(old) => old.name != name || old.rssi != rssi,
        }
    }

    fn devices(&self) -> Vec<DiscoveredDevice> {
        let mut all: Vec<DiscoveredDevice> = self
            .ble_visible
            .values()
            .chain(self.mdns.values())
            .cloned()
            .collect();
        all.extend(self.serial.iter().cloned());
        all.sort_by(|a, b| a.name.cmp(&b.name));
        all.dedup_by(|a, b| a.address == b.address);
        all
    }
}
