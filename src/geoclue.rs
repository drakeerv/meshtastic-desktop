//! Host geolocation through GeoClue2 over D-Bus.
//!
//! GeoClue lives on the system bus. We ask its manager for a client, start it,
//! wait for a single `LocationUpdated` signal, read the fix, and stop the
//! client. Everything is best effort: if GeoClue is not installed or refuses
//! the request, the caller gets a readable error rather than a panic.

use std::time::Duration;

use futures::StreamExt;
use zbus::Proxy;
use zbus::zvariant::OwnedObjectPath;

const SERVICE: &str = "org.freedesktop.GeoClue2";
const MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";
const MANAGER_IFACE: &str = "org.freedesktop.GeoClue2.Manager";
const CLIENT_IFACE: &str = "org.freedesktop.GeoClue2.Client";
const LOCATION_IFACE: &str = "org.freedesktop.GeoClue2.Location";

/// How long to wait for a first fix before giving up.
const TIMEOUT: Duration = Duration::from_secs(15);

/// A location fix reported by the host.
#[derive(Debug, Clone, Copy)]
pub struct Fix {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
}

/// Ask GeoClue for a single location fix.
pub async fn locate() -> Result<Fix, String> {
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
    // file is not fatal, so the setter's result is ignored.
    let _ = client
        .set_property("DesktopId", "org.meshtastic.Meshtastic")
        .await;
    let _ = client.set_property("RequestedAccuracyLevel", 8u32).await;
    let _ = client.set_property("DistanceThreshold", 0u32).await;

    // Subscribe before starting so the first update cannot be missed.
    let mut updates = client
        .receive_signal("LocationUpdated")
        .await
        .map_err(|err| geoclue_error(&err))?;

    client
        .call_method("Start", &())
        .await
        .map_err(|err| geoclue_error(&err))?;

    let location_path = tokio::time::timeout(TIMEOUT, async {
        while let Some(message) = updates.next().await {
            if let Ok((_old, new)) = message
                .body()
                .deserialize::<(OwnedObjectPath, OwnedObjectPath)>()
            {
                return Ok::<OwnedObjectPath, String>(new);
            }
        }
        Err("GeoClue stopped before reporting a location".to_string())
    })
    .await
    .map_err(|_| "timed out waiting for the host location".to_string())??;

    let _ = client.call_method("Stop", &()).await;

    let location = Proxy::new(&connection, SERVICE, location_path.as_str(), LOCATION_IFACE)
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
    })
}

/// A friendlier message for the common "GeoClue is not here" failures.
fn geoclue_error(err: &zbus::Error) -> String {
    let text = err.to_string();
    if text.contains("ServiceUnknown") || text.contains("not activatable") {
        "GeoClue2 is not installed or not running".to_string()
    } else {
        format!("GeoClue error: {text}")
    }
}
