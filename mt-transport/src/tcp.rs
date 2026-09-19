//! TCP (WiFi) transport.

use std::time::Duration;

use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::stream_io::run_framed;
use crate::{TransportCommand, TransportError, TransportEvent};

/// Connect timeout for WiFi nodes.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) async fn run(
    host: &str,
    port: u16,
    cmd_rx: mpsc::Receiver<TransportCommand>,
    evt_tx: mpsc::Sender<TransportEvent>,
) -> Result<(), TransportError> {
    tracing::info!(host, port, "connecting over tcp");
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| TransportError::Timeout)?
        .map_err(|e| {
            tracing::warn!(host, port, error = %e, "tcp connect failed");
            e
        })?;
    stream.set_nodelay(true).ok();
    run_framed(stream, cmd_rx, evt_tx).await
}
