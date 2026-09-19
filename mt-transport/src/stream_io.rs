//! Shared run-loop for stream based transports (TCP, serial).
//!
//! Both transports speak the same `0x94 0xC3` framing over a byte stream,
//! so the select loop (read frames / write commands) lives here and the
//! transports only differ in how the stream is opened and prodded awake.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use mt_protocol::frame::{DecodeEvent, FrameDecoder, FrameEncoder, decode_from_radio};

use crate::{JunkLogger, TransportCommand, TransportError, TransportEvent};

/// Drive a framed stream until it fails or the command channel closes.
///
/// Emits [`TransportEvent::Connected`] once `stream` is handed over (the
/// caller performs any transport specific bring-up, like wake bytes,
/// *before* calling this).
pub async fn run_framed<S>(
    mut stream: S,
    mut cmd_rx: mpsc::Receiver<TransportCommand>,
    evt_tx: mpsc::Sender<TransportEvent>,
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let _ = evt_tx.send(TransportEvent::Connected).await;

    let encoder = FrameEncoder::default();
    let mut decoder = FrameDecoder::new();
    let mut junk = JunkLogger::default();
    let mut read_buf = vec![0u8; 4096];

    loop {
        tokio::select! {
            biased;

            maybe_cmd = cmd_rx.recv() => {
                match maybe_cmd {
                    None | Some(TransportCommand::Disconnect) => {
                        // Best-effort polite goodbye, then close.
                        let goodbye = mt_protocol::builders::disconnect();
                        let _ = stream.write_all(&encoder.encode_to_radio(&goodbye)).await;
                        let _ = stream.shutdown().await;
                        return Ok(());
                    }
                    Some(TransportCommand::Send(msg)) => {
                        let bytes = encoder.encode_to_radio(&msg);
                        stream.write_all(&bytes).await?;
                    }
                    Some(TransportCommand::BlePasskey(_)) => {}
                }
            }

            read = stream.read(&mut read_buf) => {
                let n = read?;
                if n == 0 {
                    junk.flush(&evt_tx);
                    return Err(TransportError::Closed);
                }
                let mut events = Vec::new();
                decoder.feed(&read_buf[..n], &mut events);
                for event in events {
                    match event {
                        DecodeEvent::Frame(payload) => match decode_from_radio(&payload) {
                            Ok(msg) => {
                                if evt_tx.send(TransportEvent::FromRadio(msg)).await.is_err() {
                                    return Err(TransportError::Shutdown);
                                }
                            }
                            Err(e) => {
                                tracing::debug!(error = %e, "dropping undecodable FromRadio frame");
                            }
                        },
                        DecodeEvent::Junk(byte) => junk.push(byte, &evt_tx),
                    }
                }
            }
        }
    }
}
