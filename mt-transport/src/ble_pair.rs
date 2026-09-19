//! BlueZ pairing agent for BLE nodes that require a passkey.
//!
//! Meshtastic nodes in `RANDOM_PIN` mode display a six-digit passkey on their
//! screen that the host must enter before the link is encrypted. btleplug has
//! no pairing API, so we register an `org.bluez.Agent1` with BlueZ, ask it to
//! use us as the default agent, and call `Device1.Pair()`. When BlueZ asks for
//! the passkey we surface a [`TransportEvent::BlePairingRequest`] to the UI and
//! return the digits the user types.
//!
//! Nothing here is Meshtastic specific; it is the standard BlueZ pairing
//! dance, which the mobile clients get for free from their OS.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, mpsc, oneshot};
use zbus::zvariant::ObjectPath;

use crate::{TransportCommand, TransportError, TransportEvent};

/// Object path our pairing agent is exported at.
const AGENT_PATH: &str = "/org/meshtastic/Agent1";

/// The BlueZ agent capabilities string: we can both display and type a code,
/// which is what a passkey-entry device needs.
const AGENT_CAPABILITY: &str = "KeyboardDisplay";

/// How many times to re-run pairing before giving up. Each attempt gives the
/// user another passkey window without tearing the transport down.
const PAIR_ATTEMPTS: u32 = 4;

/// Ensure the device with `address` is bonded, pairing it if necessary.
///
/// Returns once BlueZ reports the device paired, or immediately if it already
/// is. `cmd_rx` is polled while pairing so the UI can supply the passkey.
pub(crate) async fn ensure_paired(
    address: &str,
    events: &mpsc::Sender<TransportEvent>,
    cmd_rx: &mut mpsc::Receiver<TransportCommand>,
) -> Result<(), TransportError> {
    let connection = zbus::Connection::system().await.map_err(ble)?;

    let Some((device_path, paired)) = find_device(&connection, address).await? else {
        // Not in BlueZ's object tree yet; nothing to pair against.
        return Ok(());
    };
    if paired {
        return Ok(());
    }

    tracing::info!(address, "device is not bonded; requesting passkey");
    let waiter: Arc<Mutex<Option<oneshot::Sender<u32>>>> = Arc::new(Mutex::new(None));
    let agent = PairingAgent {
        waiter: waiter.clone(),
        events: events.clone(),
        address: address.to_string(),
    };
    connection
        .object_server()
        .at(AGENT_PATH, agent)
        .await
        .map_err(ble)?;

    let agent_path = ObjectPath::try_from(AGENT_PATH).map_err(ble)?;
    let manager = zbus::Proxy::new(
        &connection,
        "org.bluez",
        "/org/bluez",
        "org.bluez.AgentManager1",
    )
    .await
    .map_err(ble)?;
    manager
        .call_method("RegisterAgent", &(agent_path.clone(), AGENT_CAPABILITY))
        .await
        .map_err(ble)?;
    // Best effort: another agent may already be default.
    let _ = manager
        .call_method("RequestDefaultAgent", &(agent_path.clone(),))
        .await;

    let device = zbus::Proxy::new(
        &connection,
        "org.bluez",
        device_path.as_str(),
        "org.bluez.Device1",
    )
    .await
    .map_err(ble)?;

    let outcome = {
        // Give the user several chances: each `Pair` starts a fresh passkey
        // window, and a slow entry just times out on the device side. We keep
        // the transport alive and re-prompt rather than dropping to a
        // reconnect.
        let mut outcome = Err(TransportError::Ble("pairing did not start".into()));
        for attempt in 1..=PAIR_ATTEMPTS {
            match pair_with_passkeys(&device, &waiter, cmd_rx).await {
                Ok(()) => {
                    outcome = Ok(());
                    break;
                }
                Err(TransportError::Closed) => {
                    // The user cancelled; stop immediately.
                    outcome = Err(TransportError::Closed);
                    break;
                }
                Err(error) => {
                    tracing::warn!(attempt, %error, "ble pairing attempt failed");
                    outcome = Err(error);
                    if attempt < PAIR_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(800)).await;
                    }
                }
            }
        }
        outcome
    };

    let _ = manager.call_method("UnregisterAgent", &(agent_path,)).await;

    match &outcome {
        Ok(()) => tracing::info!(address, "ble pairing complete"),
        Err(error) => tracing::warn!(address, %error, "ble pairing failed"),
    }
    outcome
}

/// Run `Device1.Pair`, pumping passkeys from the UI into the agent.
async fn pair_with_passkeys(
    device: &zbus::Proxy<'_>,
    waiter: &Arc<Mutex<Option<oneshot::Sender<u32>>>>,
    cmd_rx: &mut mpsc::Receiver<TransportCommand>,
) -> Result<(), TransportError> {
    let pair = device.call_method("Pair", &());
    tokio::pin!(pair);

    loop {
        tokio::select! {
            result = &mut pair => {
                return result.map(|_| ()).map_err(ble);
            }
            command = cmd_rx.recv() => match command {
                Some(TransportCommand::BlePasskey(passkey)) => {
                    tracing::debug!(passkey, "ble agent received passkey");
                    if let Some(sender) = waiter.lock().await.take() {
                        let _ = sender.send(passkey);
                    }
                }
                Some(TransportCommand::Send(_)) => {
                    // Commands can arrive while pairing; the link is not up
                    // yet, so there is nowhere to send them. Drop them.
                }
                Some(TransportCommand::Disconnect) | None => {
                    // Aborting drops the waiter, which fails the agent call.
                    waiter.lock().await.take();
                    let _ = device.call_method("CancelPairing", &()).await;
                    return Err(TransportError::Closed);
                }
            },
        }
    }
}

/// Find the `org.bluez.Device1` object for `address` and whether it is paired.
async fn find_device(
    connection: &zbus::Connection,
    address: &str,
) -> Result<Option<(zbus::zvariant::OwnedObjectPath, bool)>, TransportError> {
    let manager = zbus::fdo::ObjectManagerProxy::new(connection, "org.bluez", "/")
        .await
        .map_err(ble)?;
    let objects = manager.get_managed_objects().await.map_err(ble)?;

    for (path, interfaces) in objects {
        let Some(properties) = interfaces
            .iter()
            .find(|(name, _)| name.as_str() == "org.bluez.Device1")
            .map(|(_, properties)| properties)
        else {
            continue;
        };
        let matches = properties
            .get("Address")
            .and_then(|value| value.downcast_ref::<&str>().ok())
            .map(|found| found.eq_ignore_ascii_case(address))
            .unwrap_or(false);
        if matches {
            let paired = properties
                .get("Paired")
                .and_then(|value| value.downcast_ref::<bool>().ok())
                .unwrap_or(false);
            return Ok(Some((path, paired)));
        }
    }
    Ok(None)
}

/// The exported `org.bluez.Agent1` implementation.
struct PairingAgent {
    waiter: Arc<Mutex<Option<oneshot::Sender<u32>>>>,
    events: mpsc::Sender<TransportEvent>,
    address: String,
}

impl PairingAgent {
    /// Ask the UI for the passkey and wait for it.
    async fn ask(&self) -> zbus::fdo::Result<u32> {
        tracing::debug!("ble agent asking ui for passkey");
        let (sender, receiver) = oneshot::channel();
        *self.waiter.lock().await = Some(sender);
        if self
            .events
            .send(TransportEvent::BlePairingRequest {
                address: self.address.clone(),
            })
            .await
            .is_err()
        {
            return Err(zbus::fdo::Error::Failed("ui closed".into()));
        }
        let result = receiver
            .await
            .map_err(|_| zbus::fdo::Error::AuthFailed("pairing cancelled".into()));
        tracing::debug!(ok = result.is_ok(), "ble agent passkey resolved");
        result
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl PairingAgent {
    async fn request_passkey(&self, _device: ObjectPath<'_>) -> zbus::fdo::Result<u32> {
        tracing::debug!("agent1 request_passkey");
        self.ask().await
    }

    async fn request_pin_code(&self, _device: ObjectPath<'_>) -> zbus::fdo::Result<String> {
        tracing::debug!("agent1 request_pin_code");
        Ok(self.ask().await?.to_string())
    }

    async fn request_confirmation(&self, _device: ObjectPath<'_>, _passkey: u32) {
        tracing::debug!("agent1 request_confirmation");
    }

    async fn request_authorization(&self, _device: ObjectPath<'_>) {
        tracing::debug!("agent1 request_authorization");
    }

    async fn authorize_service(&self, _device: ObjectPath<'_>, _uuid: String) {
        tracing::debug!("agent1 authorize_service");
    }

    async fn display_passkey(&self, _device: ObjectPath<'_>, _passkey: u32, _entered: u16) {
        tracing::debug!("agent1 display_passkey");
    }

    async fn display_pin_code(&self, _device: ObjectPath<'_>, _pincode: String) {
        tracing::debug!("agent1 display_pin_code");
    }

    async fn cancel(&self) {
        tracing::debug!("agent1 cancel");
    }

    async fn release(&self) {
        tracing::debug!("agent1 release");
    }
}

/// Convert a zbus/zvariant error into the transport error type.
fn ble(error: impl std::fmt::Display) -> TransportError {
    TransportError::Ble(error.to_string())
}
