//! Host location, tried through a chain of providers.
//!
//! Sharing walks the providers in order and forwards fixes until the
//! consumer stops listening:
//!
//! 1. **GeoClue2** over D-Bus, the desktop standard. Streams while the host
//!    moves (or every 30 s at most).
//! 2. **gpsd**, for a locally attached GPS receiver, polled every 30 s.
//! 3. **IP geolocation** over HTTP, which needs no setup but is city-level
//!    and shares the public IP with a third party, so it is opt-in. Sent
//!    once: a city does not move.
//!
//! Manual entry lives in the UI and does not go through here.

use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::mpsc;

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

/// Stream fixes from the first provider that works.
///
/// `allow_ip` gates the IP fallback, which contacts an external service.
pub async fn stream(allow_ip: bool, tx: mpsc::Sender<Result<Fix, String>>) {
    let mut problems = Vec::new();

    match crate::geoclue::stream(tx.clone()).await {
        Ok(()) => return,
        Err(err) => problems.push(err),
    }

    match gpsd::stream(tx.clone()).await {
        Ok(()) => return,
        Err(err) => problems.push(err),
    }

    if allow_ip {
        match ip::locate().await {
            Ok((latitude, longitude, accuracy)) => {
                let _ = tx
                    .send(Ok(Fix {
                        latitude,
                        longitude,
                        accuracy,
                        source: "IP address",
                    }))
                    .await;
                return;
            }
            Err(err) => problems.push(err),
        }
    } else {
        problems.push("IP geolocation is off".to_string());
    }

    let _ = tx
        .send(Err(format!(
            "no location provider worked ({})",
            problems.join("; ")
        )))
        .await;
}

/// A locally attached GPS receiver, read through gpsd.
mod gpsd {
    use super::*;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    const SOCKET: &str = "/run/gpsd.sock";
    const TCP: &str = "127.0.0.1:2947";

    /// gpsd reports a fix about once per second; forward at most this often.
    const SEND_INTERVAL: Duration = Duration::from_secs(30);
    /// How long to wait for the next sentence before giving up.
    const READ_TIMEOUT: Duration = Duration::from_secs(15);

    /// Stream fixes from a GPS receiver until the consumer stops listening.
    pub async fn stream(tx: mpsc::Sender<Result<Fix, String>>) -> Result<(), String> {
        if let Ok(stream) = tokio::net::UnixStream::connect(SOCKET).await {
            return read_stream(stream, tx).await;
        }
        match tokio::net::TcpStream::connect(TCP).await {
            Ok(stream) => read_stream(stream, tx).await,
            Err(err) => Err(format!("gpsd not reachable: {err}")),
        }
    }

    async fn read_stream<S>(stream: S, tx: mpsc::Sender<Result<Fix, String>>) -> Result<(), String>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let (read, mut write) = tokio::io::split(stream);
        write
            .write_all(b"?WATCH={\"enable\":true,\"json\":true}\n")
            .await
            .map_err(|err| format!("gpsd write failed: {err}"))?;
        let mut lines = BufReader::new(read).lines();
        let mut last_sent: Option<Instant> = None;

        loop {
            tokio::select! {
                _ = tx.closed() => return Ok(()),
                line = tokio::time::timeout(READ_TIMEOUT, lines.next_line()) => {
                    let line = line
                        .map_err(|_| "timed out waiting for gpsd".to_string())?
                        .map_err(|err| format!("gpsd read failed: {err}"))?
                        .ok_or_else(|| "gpsd closed the connection".to_string())?;
                    let Some((latitude, longitude, accuracy)) = parse_tpv(&line) else {
                        continue;
                    };
                    let due = last_sent
                        .map(|at| at.elapsed() >= SEND_INTERVAL)
                        .unwrap_or(true);
                    if due {
                        last_sent = Some(Instant::now());
                        if tx
                            .send(Ok(Fix {
                                latitude,
                                longitude,
                                accuracy,
                                source: "gpsd",
                            }))
                            .await
                            .is_err()
                        {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

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

    pub async fn locate() -> Result<(f64, f64, f64), String> {
        let response = reqwest::Client::new()
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
