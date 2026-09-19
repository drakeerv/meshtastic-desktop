//! USB serial transport.
//!
//! Uses 115200 8N1 with DTR + RTS asserted (many boards gate their USB
//! bridge or MCU console on those lines) and writes the standard wake-up
//! byte run before the first frame so ESP32 based nodes exit light sleep.

use std::time::Duration;

use serialport::SerialPort as _;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio_serial::SerialPortBuilderExt;

use mt_protocol::constants::{SERIAL_BAUD, WAKE_BYTES};

use crate::stream_io::run_framed;
use crate::{TransportCommand, TransportError, TransportEvent};

pub(crate) async fn run(
    path: &str,
    cmd_rx: mpsc::Receiver<TransportCommand>,
    evt_tx: mpsc::Sender<TransportEvent>,
) -> Result<(), TransportError> {
    tracing::info!(path, "opening serial port");

    let builder = tokio_serial::new(path, SERIAL_BAUD);
    let mut stream = tokio::task::spawn_blocking(move || builder.open_native_async())
        .await
        .map_err(|e| TransportError::Serial(format!("join error: {e}")))?
        .map_err(|e| {
            tracing::warn!(path, error = %e, "serial open failed");
            TransportError::Serial(format!("{e}"))
        })?;

    // Signal the MCU: we are here, and we want its console.
    if let Err(e) = stream.write_data_terminal_ready(true) {
        tracing::debug!(error = %e, "could not set DTR");
    }
    if let Err(e) = stream.write_request_to_send(true) {
        tracing::debug!(error = %e, "could not set RTS");
    }

    // Wake bytes: repeated 0x94. Harmless to a woken device (the frame
    // decoder treats them as padding) and required for sleeping ones.
    stream.write_all(&WAKE_BYTES).await?;
    // Give slow boards a beat to leave deep sleep before the handshake.
    tokio::time::sleep(Duration::from_millis(120)).await;

    run_framed(stream, cmd_rx, evt_tx).await
}
