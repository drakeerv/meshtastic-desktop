//! Host location, tried through a chain of providers.
//!
//! "Use host location" walks the providers in order and returns the first fix:
//!
//! 1. **GeoClue2** over D-Bus, the desktop standard.
//! 2. **gpsd**, for a locally attached GPS receiver.
//! 3. **IP geolocation** over HTTP, which needs no setup but is city-level and
//!    shares the public IP with a third party, so it is opt-in.
//!
//! Manual entry lives in the UI and does not go through here.

use std::time::Duration;

use serde::Deserialize;

/// A location fix from whichever provider answered.
#[derive(Debug, Clone, Copy)]
pub struct Fix {
    pub latitude: f64,
    pub longitude: f64,
    /// Estimated error radius in metres, or 0 when unknown.
    pub accuracy: f64,
    /// A short human-readable provider name for the notice.
    pub source: &'static str,
}

/// Try each provider in turn and return the first fix.
///
/// `allow_ip` gates the IP fallback, which contacts an external service.
pub async fn locate(client: reqwest::Client, allow_ip: bool) -> Result<Fix, String> {
    let mut problems = Vec::new();

    match crate::geoclue::locate().await {
        Ok(fix) => {
            return Ok(Fix {
                latitude: fix.latitude,
                longitude: fix.longitude,
                accuracy: fix.accuracy,
                source: "GeoClue",
            });
        }
        Err(err) => problems.push(err),
    }

    match gpsd::locate().await {
        Ok((latitude, longitude, accuracy)) => {
            return Ok(Fix {
                latitude,
                longitude,
                accuracy,
                source: "gpsd",
            });
        }
        Err(err) => problems.push(err),
    }

    if allow_ip {
        match ip::locate(&client).await {
            Ok((latitude, longitude, accuracy)) => {
                return Ok(Fix {
                    latitude,
                    longitude,
                    accuracy,
                    source: "IP address",
                });
            }
            Err(err) => problems.push(err),
        }
    } else {
        problems.push("IP geolocation is off".to_string());
    }

    Err(format!(
        "no location provider worked ({})",
        problems.join("; ")
    ))
}

/// A locally attached GPS receiver, read through gpsd.
mod gpsd {
    use super::*;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const SOCKET: &str = "/run/gpsd.sock";
    const TCP: &str = "127.0.0.1:2947";

    /// Ask gpsd for a fix, preferring its Unix socket and falling back to TCP.
    pub async fn locate() -> Result<(f64, f64, f64), String> {
        if let Ok(stream) = tokio::net::UnixStream::connect(SOCKET).await {
            return read_fix(stream).await;
        }
        match tokio::net::TcpStream::connect(TCP).await {
            Ok(stream) => read_fix(stream).await,
            Err(err) => Err(format!("gpsd not reachable: {err}")),
        }
    }

    async fn read_fix<S>(stream: S) -> Result<(f64, f64, f64), String>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let (read, mut write) = tokio::io::split(stream);
        write
            .write_all(b"?WATCH={\"enable\":true,\"json\":true}\n")
            .await
            .map_err(|err| format!("gpsd write failed: {err}"))?;

        let mut lines = BufReader::new(read).lines();
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            let line = tokio::time::timeout_at(deadline, lines.next_line())
                .await
                .map_err(|_| "timed out waiting for gpsd".to_string())?
                .map_err(|err| format!("gpsd read failed: {err}"))?
                .ok_or_else(|| "gpsd closed the connection".to_string())?;

            if let Some(fix) = parse_tpv(&line) {
                let _ = write.write_all(b"?WATCH={\"enable\":false}\n").await;
                return Ok(fix);
            }
        }
    }

    /// How long to wait for gpsd to report a fix.
    const TIMEOUT: Duration = Duration::from_secs(15);

    /// Pull a fix out of a gpsd `TPV` sentence, if it has one.
    pub(super) fn parse_tpv(line: &str) -> Option<(f64, f64, f64)> {
        #[derive(Deserialize)]
        struct Tpv {
            class: String,
            #[serde(default)]
            mode: i32,
            lat: Option<f64>,
            lon: Option<f64>,
            epx: Option<f64>,
            epy: Option<f64>,
        }

        let tpv: Tpv = serde_json::from_str(line).ok()?;
        // mode 2 is a 2D fix, 3 is 3D; below that there is no position.
        if tpv.class != "TPV" || tpv.mode < 2 {
            return None;
        }
        let accuracy = tpv.epx.unwrap_or(0.0).max(tpv.epy.unwrap_or(0.0));
        Some((tpv.lat?, tpv.lon?, accuracy))
    }
}

/// City-level geolocation from the public IP address, via a keyless service.
mod ip {
    use super::*;

    /// A nominal error radius; IP geolocation is only good to a city.
    const ACCURACY: f64 = 25_000.0;

    #[derive(Deserialize)]
    struct WhoIs {
        latitude: Option<f64>,
        longitude: Option<f64>,
    }

    pub async fn locate(client: &reqwest::Client) -> Result<(f64, f64, f64), String> {
        let response = client
            .get("https://ipwho.is/")
            .send()
            .await
            .map_err(|err| format!("IP lookup failed: {err}"))?;
        if !response.status().is_success() {
            return Err(format!("IP lookup returned {}", response.status()));
        }
        let body = response
            .text()
            .await
            .map_err(|err| format!("IP lookup reply was unreadable: {err}"))?;

        let (latitude, longitude) = parse(&body)?;
        Ok((latitude, longitude, ACCURACY))
    }

    /// Read coordinates out of an `ipwho.is` reply.
    pub(super) fn parse(body: &str) -> Result<(f64, f64), String> {
        let who: WhoIs = serde_json::from_str(body)
            .map_err(|err| format!("IP lookup reply was invalid: {err}"))?;
        match (who.latitude, who.longitude) {
            (Some(latitude), Some(longitude)) => Ok((latitude, longitude)),
            _ => Err("IP lookup returned no coordinates".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_gpsd_fix() {
        let line = r#"{"class":"TPV","mode":3,"lat":37.2296,"lon":-80.4139,"epx":8.1,"epy":12.4}"#;
        let (lat, lon, accuracy) = gpsd::parse_tpv(line).expect("fix");
        assert!((lat - 37.2296).abs() < 1e-9);
        assert!((lon + 80.4139).abs() < 1e-9);
        assert!((accuracy - 12.4).abs() < 1e-9);
    }

    #[test]
    fn ignores_gpsd_without_a_fix() {
        assert!(gpsd::parse_tpv(r#"{"class":"TPV","mode":1}"#).is_none());
        assert!(gpsd::parse_tpv(r#"{"class":"VERSION","release":"3.27"}"#).is_none());
        assert!(gpsd::parse_tpv("not json").is_none());
    }

    #[test]
    fn parses_an_ip_lookup_reply() {
        let body = r#"{"ip":"1.2.3.4","latitude":37.2296,"longitude":-80.4139}"#;
        assert_eq!(ip::parse(body), Ok((37.2296, -80.4139)));
        assert!(ip::parse(r#"{"ip":"1.2.3.4"}"#).is_err());
    }
}
