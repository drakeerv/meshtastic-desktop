//! Bluetooth Low Energy transport.
//!
//! Implements the official GATT protocol:
//!
//! - `TORADIO`   (write)      - client pushes `ToRadio` protobufs
//! - `FROMRADIO` (read)       - client drains until the read comes back empty
//! - `FROMNUM`   (notify)     - signals that `FROMRADIO` has data
//! - `LOGRADIO`  (notify)     - optional device log stream
//!
//! Unlike TCP/serial there is no `0x94 0xC3` framing here: protobufs are
//! exchanged whole over the characteristics.

use std::str::FromStr;
use std::time::Duration;

use btleplug::api::{
    BDAddr, Central as _, CharPropFlags, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use tokio::sync::mpsc;

use mt_protocol::constants::{
    BLE_FROMNUM_UUID, BLE_FROMRADIO_UUID, BLE_LOGRADIO_UUID, BLE_TORADIO_UUID,
};
use mt_protocol::frame::{decode_from_radio, encode_protobuf};

use crate::{JunkLogger, TransportCommand, TransportError, TransportEvent};

/// How long to spend looking for the peripheral before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How often to check link liveness (notification silence is normal when
/// the mesh is quiet, so we poll `is_connected`).
const LIVENESS_POLL: Duration = Duration::from_secs(5);

struct GattCharacteristics {
    toradio: btleplug::api::Characteristic,
    fromradio: btleplug::api::Characteristic,
}

pub(crate) async fn run(
    mac: &str,
    mut cmd_rx: mpsc::Receiver<TransportCommand>,
    evt_tx: mpsc::Sender<TransportEvent>,
) -> Result<(), TransportError> {
    tracing::info!(mac, "connecting over ble");

    let manager = Manager::new().await?;
    let mut adapters = manager.adapters().await?;
    let adapter: Adapter = adapters
        .drain(..)
        .next()
        .ok_or_else(|| TransportError::NotFound("no bluetooth adapter".into()))?;

    let target =
        BDAddr::from_str(mac).map_err(|_| TransportError::InvalidAddress(mac.to_string()))?;

    let peripheral = find_peripheral(&adapter, target).await?;

    // A node in RANDOM_PIN mode shows a six-digit passkey on its screen and
    // refuses GATT reads until bonded. btleplug cannot pair, so drive BlueZ
    // ourselves and relay the passkey from the UI.
    crate::ble_pair::ensure_paired(mac, &evt_tx, &mut cmd_rx).await?;

    connect_and_setup(&peripheral).await?;

    let _ = evt_tx.send(TransportEvent::Connected).await;
    drive(peripheral, &mut cmd_rx, &evt_tx).await
}

/// Locate the peripheral for `target`, running a scan if it is not already
/// known to the adapter.
async fn find_peripheral(adapter: &Adapter, target: BDAddr) -> Result<Peripheral, TransportError> {
    // Fast path: already cached by the adapter.
    for p in adapter.peripherals().await? {
        if p.address() == target {
            return Ok(p);
        }
    }

    // Otherwise scan for it. Scan unfiltered and match by address: BlueZ's
    // service-UUID discovery filter misses devices that are otherwise
    // visible (see the discovery module).
    tracing::debug!(?target, "ble peripheral not cached, scanning");
    adapter.start_scan(ScanFilter::default()).await?;

    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        for p in adapter.peripherals().await? {
            if p.address() == target {
                adapter.stop_scan().await.ok();
                return Ok(p);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    adapter.stop_scan().await.ok();
    Err(TransportError::NotFound(format!(
        "bluetooth device {target} not found within {CONNECT_TIMEOUT:?}"
    )))
}

/// Connect, discover services and subscribe to notification
/// characteristics.
async fn connect_and_setup(peripheral: &Peripheral) -> Result<(), TransportError> {
    peripheral.connect().await?;

    if !peripheral.is_connected().await? {
        return Err(TransportError::Closed);
    }

    // BlueZ only populates the characteristic table after an explicit
    // service discovery, and it can lag the connection slightly.
    discover_services(peripheral).await?;

    // Subscribe to FROMNUM: every notification means "FROMRADIO has data".
    wait_for_characteristics(peripheral).await?;
    let characteristics = peripheral.characteristics();
    let fromnum = characteristics
        .iter()
        .find(|c| c.uuid == BLE_FROMNUM_UUID)
        .ok_or_else(|| TransportError::NotFound("FROMNUM characteristic missing".into()))?;
    peripheral.subscribe(fromnum).await?;

    // LOGRADIO is optional.
    if let Some(log_char) = characteristics.iter().find(|c| c.uuid == BLE_LOGRADIO_UUID) {
        peripheral.subscribe(log_char).await.ok();
    }

    tracing::info!("ble gatt ready");
    Ok(())
}

/// Run GATT service discovery, retrying while the stack resolves services.
async fn discover_services(peripheral: &Peripheral) -> Result<(), TransportError> {
    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
    loop {
        match peripheral.discover_services().await {
            Ok(()) => return Ok(()),
            Err(error) if tokio::time::Instant::now() < deadline => {
                tracing::debug!(%error, "gatt discovery not ready, retrying");
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

/// Waits until the GATT table contains our characteristics.
async fn wait_for_characteristics(
    peripheral: &Peripheral,
) -> Result<GattCharacteristics, TransportError> {
    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
    loop {
        let characteristics = peripheral.characteristics();
        let toradio = characteristics
            .iter()
            .find(|c| c.uuid == BLE_TORADIO_UUID && c.properties.contains(CharPropFlags::WRITE));
        let fromradio = characteristics
            .iter()
            .find(|c| c.uuid == BLE_FROMRADIO_UUID && c.properties.contains(CharPropFlags::READ));

        if let (Some(toradio), Some(fromradio)) = (toradio, fromradio) {
            return Ok(GattCharacteristics {
                toradio: toradio.clone(),
                fromradio: fromradio.clone(),
            });
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(TransportError::NotFound(
                "Meshtastic GATT characteristics missing".into(),
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// The main BLE loop: writes commands to TORADIO, reacts to FROMNUM
/// notifications by draining FROMRADIO, and watches the connection state.
async fn drive(
    peripheral: Peripheral,
    cmd_rx: &mut mpsc::Receiver<TransportCommand>,
    evt_tx: &mpsc::Sender<TransportEvent>,
) -> Result<(), TransportError> {
    let chars = wait_for_characteristics(&peripheral).await?;
    let mut notifications = Box::pin(peripheral.notifications().await?);
    let mut junk = JunkLogger::default();
    let mut liveness = tokio::time::interval(LIVENESS_POLL);
    liveness.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Drain anything the device queued before we subscribed.
    drain_from_radio(&peripheral, &chars, evt_tx, &mut junk).await?;

    loop {
        tokio::select! {
            biased;

            maybe_cmd = cmd_rx.recv() => {
                match maybe_cmd {
                    None | Some(TransportCommand::Disconnect) => {
                        let _ = peripheral.disconnect().await;
                        return Ok(());
                    }
                    Some(TransportCommand::Send(msg)) => {
                        let bytes = encode_protobuf(&msg);
                        tracing::debug!(len = bytes.len(), "ble write ToRadio");
                        peripheral
                            .write(&chars.toradio, &bytes, WriteType::WithResponse)
                            .await?;
                        // The firmware gates FROMNUM notifications behind
                        // STATE_SEND_PACKETS, so during the config handshake a
                        // write queues data without any notification arriving.
                        // Poll proactively after every write, like the official
                        // clients do.
                        drain_from_radio(&peripheral, &chars, evt_tx, &mut junk).await?;
                    }
                    Some(TransportCommand::BlePasskey(_)) => {
                        // Pairing is already done by the time the link is up.
                    }
                }
            }

            notification = notifications.next() => {
                let Some(n) = notification else {
                    tracing::warn!("ble notification stream ended");
                    return Err(TransportError::Closed);
                };
                if n.uuid == BLE_FROMNUM_UUID {
                    tracing::debug!("ble FROMNUM notification, draining");
                    drain_from_radio(&peripheral, &chars, evt_tx, &mut junk).await?;
                } else if n.uuid == BLE_LOGRADIO_UUID {
                    let text = String::from_utf8_lossy(&n.value);
                    for line in text.lines().filter(|l| !l.trim().is_empty()) {
                        let _ = evt_tx.try_send(TransportEvent::DeviceLog(line.to_string()));
                    }
                }
            }

            _ = liveness.tick() => {
                junk.flush(evt_tx);
                if !peripheral.is_connected().await? {
                    tracing::info!("ble peripheral disconnected (liveness check)");
                    return Err(TransportError::Closed);
                }
            }
        }
    }
}

/// Read `FROMRADIO` repeatedly until it returns an empty payload, pushing
/// each decoded message into the event channel.
async fn drain_from_radio(
    peripheral: &Peripheral,
    chars: &GattCharacteristics,
    evt_tx: &mpsc::Sender<TransportEvent>,
    junk: &mut JunkLogger,
) -> Result<(), TransportError> {
    loop {
        let payload = peripheral.read(&chars.fromradio).await?;
        if payload.is_empty() {
            return Ok(());
        }
        match decode_from_radio(&payload) {
            Ok(msg) => {
                if evt_tx
                    .send(TransportEvent::FromRadio(Box::new(msg)))
                    .await
                    .is_err()
                {
                    return Err(TransportError::Shutdown);
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "undecodable FROMRADIO payload");
                for &b in &payload {
                    junk.push(b, evt_tx);
                }
            }
        }
    }
}
