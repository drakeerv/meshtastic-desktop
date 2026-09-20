//! Host geolocation through GeoClue2 over D-Bus, as a long-lived stream.
//!
//! GeoClue lives on the system bus. We ask its manager for a client, set the
//! desktop id and update thresholds, start it, and forward every
//! `LocationUpdated` signal as a fix. Keeping one client alive for the whole
//! sharing session means the authorization agent is consulted once per app
//! run rather than once per fix.

use futures::StreamExt;
use tokio::sync::mpsc;
use zbus::Proxy;
use zbus::zvariant::OwnedObjectPath;

use crate::location::Fix;

const SERVICE: &str = "org.freedesktop.GeoClue2";
const MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";
const MANAGER_IFACE: &str = "org.freedesktop.GeoClue2.Manager";
const CLIENT_IFACE: &str = "org.freedesktop.GeoClue2.Client";
const LOCATION_IFACE: &str = "org.freedesktop.GeoClue2.Location";

/// Emit at most one fix per this many seconds...
const TIME_THRESHOLD: u32 = 30;
/// ...or as soon as the host has moved at least this many metres.
const DISTANCE_THRESHOLD: u32 = 25;

/// Stream fixes until the consumer stops listening.
///
/// Returns `Ok(())` once the receiver is gone (or the caller should move on
/// to another provider); an error means GeoClue could not be set up at all.
pub async fn stream(tx: mpsc::Sender<Result<Fix, String>>) -> Result<(), String> {
    let connection = zbus::Connection::system()
        .await
        .map_err(|err| format!("D-Bus unavailable: {err}"))?;

    let manager = Proxy::new(&connection, SERVICE, MANAGER_PATH, MANAGER_IFACE)
        .await
        .map_err(|err| geoclue_error(&err))?;

    let client_path: OwnedObjectPath = manager
        .call("GetClient", &())
        .await
        .map_err(|err| geoclue_error(&err))?;

    let client = Proxy::new(&connection, SERVICE, client_path.as_str(), CLIENT_IFACE)
        .await
        .map_err(|err| geoclue_error(&err))?;

    // The desktop id lets GeoClue attribute the request; a missing desktop
    // file is not fatal for the connection, only for agent authorization.
    let _ = client.set_property("DesktopId", crate::APP_ID).await;
    let _ = client.set_property("RequestedAccuracyLevel", 8u32).await;
    let _ = client.set_property("TimeThreshold", TIME_THRESHOLD).await;
    let _ = client
        .set_property("DistanceThreshold", DISTANCE_THRESHOLD)
        .await;

    // Subscribe before starting so the first update cannot be missed.
    let mut updates = client
        .receive_signal("LocationUpdated")
        .await
        .map_err(|err| geoclue_error(&err))?;

    client
        .call_method("Start", &())
        .await
        .map_err(|err| geoclue_error(&err))?;
    tracing::info!("geoclue location stream started");

    let outcome = loop {
        tokio::select! {
            _ = tx.closed() => break Ok(()),
            signal = updates.next() => {
                let Some(signal) = signal else {
                    break Err("GeoClue stopped before reporting a location".to_string());
                };
                let Ok((_old, new)) = signal
                    .body()
                    .deserialize::<(OwnedObjectPath, OwnedObjectPath)>()
                else {
                    continue;
                };
                match read_fix(&connection, &new).await {
                    Ok(fix) => {
                        if tx.send(Ok(fix)).await.is_err() {
                            break Ok(());
                        }
                    }
                    Err(err) => tracing::debug!(%err, "geoclue location read failed"),
                }
            }
        }
    };

    let _ = client.call_method("Stop", &()).await;
    outcome
}

/// Read the coordinates out of a GeoClue location object.
async fn read_fix(connection: &zbus::Connection, path: &OwnedObjectPath) -> Result<Fix, String> {
    let location = Proxy::new(connection, SERVICE, path.as_str(), LOCATION_IFACE)
        .await
        .map_err(|err| geoclue_error(&err))?;

    let latitude: f64 = location
        .get_property("Latitude")
        .await
        .map_err(|err| geoclue_error(&err))?;
    let longitude: f64 = location
        .get_property("Longitude")
        .await
        .map_err(|err| geoclue_error(&err))?;
    let accuracy: f64 = location.get_property("Accuracy").await.unwrap_or(0.0);

    Ok(Fix {
        latitude,
        longitude,
        accuracy,
        source: "GeoClue",
    })
}

/// A friendlier message for the common "GeoClue is not here" failures.
fn geoclue_error(err: &zbus::Error) -> String {
    let text = err.to_string();
    if text.contains("ServiceUnknown") || text.contains("not activatable") {
        "GeoClue2 is not installed or not running".to_string()
    } else if text.contains("not authorized") {
        "host denied location access; run scripts/install-desktop.sh so the \
         authorization agent can identify the app"
            .to_string()
    } else {
        format!("GeoClue error: {text}")
    }
}
