//! The `0x94 0xC3` stream frame codec used by the TCP and serial transports.
//!
//! Wire format:
//!
//! ```text
//! +--------+--------+--------+--------+===============+
//! |  0x94  |  0xC3  | len_hi | len_lo |   payload     |
//! +--------+--------+--------+--------+===============+
//! ```
//!
//! The decoder is *desync tolerant*: devices sometimes emit boot logs or
//! debug output on the same stream. Any byte that cannot be part of a valid
//! frame header is surfaced as [`DecodeEvent::Junk`] instead of poisoning
//! the stream, and parsing resynchronizes on the next `0x94 0xC3` marker.
//! Repeated `0x94` bytes (wake-up sequences echoed back, or sent by us) are
//! treated as padding and skipped.

use crate::constants::{MAX_FRAME_PAYLOAD, START1, START2};

/// A single event produced by the [`FrameDecoder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeEvent {
    /// A complete frame payload (encoded protobuf bytes).
    Frame(Vec<u8>),
    /// A byte that was not part of any valid frame (typically device logs).
    Junk(u8),
}

/// Incremental, push-based decoder for the Meshtastic stream framing.
#[derive(Debug, Clone)]
pub struct FrameDecoder {
    state: State,
    /// Count of frames decoded successfully (for diagnostics).
    frames: u64,
    /// Count of junk bytes discarded (for diagnostics).
    junk: u64,
}

#[derive(Debug, Clone)]
enum State {
    /// Looking for the first start byte.
    Hunt,
    /// Saw `0x94`, waiting to see if the next byte is `0xC3`.
    Start1,
    /// Saw the full marker, waiting for the length high byte.
    LenHi,
    /// Waiting for the length low byte.
    LenLo { hi: u8 },
    /// Inside the payload of a frame.
    Body { len: usize, buf: Vec<u8> },
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self {
            state: State::Hunt,
            frames: 0,
            junk: 0,
        }
    }

    /// Feed a slice of bytes, appending decode events to `out`.
    pub fn feed(&mut self, bytes: &[u8], out: &mut Vec<DecodeEvent>) {
        for &b in bytes {
            self.feed_byte(b, out);
        }
    }

    /// Feed a single byte, appending decode events to `out`.
    pub fn feed_byte(&mut self, b: u8, out: &mut Vec<DecodeEvent>) {
        match core::mem::replace(&mut self.state, State::Hunt) {
            State::Hunt => {
                if b == START1 {
                    self.state = State::Start1;
                } else {
                    self.junk += 1;
                    out.push(DecodeEvent::Junk(b));
                }
            }
            State::Start1 => match b {
                START2 => self.state = State::LenHi,
                // Run of 0x94s: wake padding, stay in Start1.
                START1 => self.state = State::Start1,
                // The 0x94 we consumed was junk; re-examine this byte.
                _ => {
                    self.junk += 1;
                    out.push(DecodeEvent::Junk(START1));
                    self.feed_byte(b, out);
                }
            },
            State::LenHi => {
                self.state = State::LenLo { hi: b };
            }
            State::LenLo { hi } => {
                let len = ((hi as usize) << 8) | b as usize;
                if len == 0 || len > MAX_FRAME_PAYLOAD {
                    // Invalid length: this is not a frame. Junk the bytes we
                    // consumed as header and re-examine the current byte from
                    // the hunting state.
                    self.junk += 3;
                    out.push(DecodeEvent::Junk(START1));
                    out.push(DecodeEvent::Junk(START2));
                    out.push(DecodeEvent::Junk(hi));
                    self.feed_byte(b, out);
                } else {
                    self.state = State::Body {
                        len,
                        buf: Vec::with_capacity(len),
                    };
                }
            }
            State::Body { len, mut buf } => {
                buf.push(b);
                if buf.len() == len {
                    self.frames += 1;
                    out.push(DecodeEvent::Frame(buf));
                } else {
                    self.state = State::Body { len, buf };
                }
            }
        }
    }

    /// Number of complete frames decoded so far.
    pub fn frames_decoded(&self) -> u64 {
        self.frames
    }

    /// Number of junk bytes discarded so far.
    pub fn junk_bytes(&self) -> u64 {
        self.junk
    }
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Encodes a payload into a wire frame, prefixing the start marker and the
/// big-endian payload length.
///
/// # Panics
///
/// Panics if `payload` exceeds [`MAX_FRAME_PAYLOAD`]; callers build protobufs
/// that are always smaller than this limit.
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    assert!(
        payload.len() <= MAX_FRAME_PAYLOAD,
        "frame payload of {} bytes exceeds the {} byte protocol limit",
        payload.len(),
        MAX_FRAME_PAYLOAD
    );
    let mut out = Vec::with_capacity(payload.len() + 4);
    out.push(START1);
    out.push(START2);
    out.push((payload.len() >> 8) as u8);
    out.push((payload.len() & 0xFF) as u8);
    out.extend_from_slice(payload);
    out
}

/// Convenience wrapper for building framed `ToRadio` messages.
#[derive(Debug, Default, Clone)]
pub struct FrameEncoder;

impl FrameEncoder {
    /// Wraps `payload` in a frame including the `0x94 0xC3` header.
    pub fn encode(&self, payload: &[u8]) -> Vec<u8> {
        encode_frame(payload)
    }

    /// Serialises a `ToRadio` protobuf and wraps it in a frame.
    pub fn encode_to_radio(&self, msg: &meshtastic_protobufs::meshtastic::ToRadio) -> Vec<u8> {
        let payload = encode_protobuf(msg);
        encode_frame(&payload)
    }
}

/// Encode a prost message to bytes.
pub fn encode_protobuf<M: prost::Message>(msg: &M) -> Vec<u8> {
    let mut buf = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut buf)
        .expect("prost encoding to Vec is infallible");
    buf
}

/// Decode a `FromRadio` protobuf from frame payload bytes.
pub fn decode_from_radio(
    bytes: &[u8],
) -> Result<meshtastic_protobufs::meshtastic::FromRadio, DecodeError> {
    use prost::Message as _;
    meshtastic_protobufs::meshtastic::FromRadio::decode(bytes).map_err(DecodeError::Protobuf)
}

/// Errors that can occur while decoding a `FromRadio` payload.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("protobuf decode failed: {0}")]
    Protobuf(#[from] prost::DecodeError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(bytes: &[u8]) -> Vec<DecodeEvent> {
        let mut d = FrameDecoder::new();
        let mut out = Vec::new();
        d.feed(bytes, &mut out);
        out
    }

    #[test]
    fn round_trips_single_frame() {
        let payload = b"hello mesh".to_vec();
        let framed = encode_frame(&payload);
        let events = collect(&framed);
        assert_eq!(events, vec![DecodeEvent::Frame(payload)]);
    }

    #[test]
    fn round_trips_max_frame() {
        let payload = vec![0xAB; MAX_FRAME_PAYLOAD];
        let events = collect(&encode_frame(&payload));
        assert_eq!(events, vec![DecodeEvent::Frame(payload)]);
    }

    #[test]
    fn splits_back_to_back_frames() {
        let a = encode_frame(b"first");
        let b = encode_frame(b"second");
        let mut stream = a;
        stream.extend_from_slice(&b);
        let events = collect(&stream);
        assert_eq!(
            events,
            vec![
                DecodeEvent::Frame(b"first".to_vec()),
                DecodeEvent::Frame(b"second".to_vec()),
            ]
        );
    }

    #[test]
    fn handles_byte_at_a_time_feeding() {
        let framed = encode_frame(b"chunked");
        let mut d = FrameDecoder::new();
        let mut out = Vec::new();
        for &b in &framed {
            d.feed_byte(b, &mut out);
        }
        assert_eq!(out, vec![DecodeEvent::Frame(b"chunked".to_vec())]);
    }

    #[test]
    fn skips_wake_padding() {
        let framed = encode_frame(b"x");
        let mut stream = vec![0x94, 0x94, 0x94, 0x94];
        stream.extend_from_slice(&framed);
        let events = collect(&stream);
        assert_eq!(events, vec![DecodeEvent::Frame(b"x".to_vec())]);
    }

    #[test]
    fn junk_between_frames_is_surfaced() {
        // Device boot log line before a frame.
        let mut stream = b"ets Jul 29 2019\r\n".to_vec();
        stream.extend_from_slice(&encode_frame(b"f"));
        let events = collect(&stream);
        assert_eq!(events.last(), Some(&DecodeEvent::Frame(b"f".to_vec())));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, DecodeEvent::Junk(_)))
                .count(),
            b"ets Jul 29 2019\r\n".len()
        );
    }

    #[test]
    fn stray_start_byte_becomes_junk() {
        // 0x94 followed by a non-marker byte: the 0x94 is junk, the byte is
        // re-examined (and itself junk).
        let events = collect(&[START1, b'q']);
        assert_eq!(
            events,
            vec![DecodeEvent::Junk(START1), DecodeEvent::Junk(b'q')]
        );
    }

    #[test]
    fn oversized_length_resyncs() {
        // Header claiming 0x1000 bytes (4096) - over the limit. The four
        // header bytes become junk and a real frame that follows still
        // decodes.
        let mut stream = vec![START1, START2, 0x10, 0x00];
        stream.extend_from_slice(&encode_frame(b"after"));
        let events = collect(&stream);
        assert_eq!(
            events,
            vec![
                DecodeEvent::Junk(START1),
                DecodeEvent::Junk(START2),
                DecodeEvent::Junk(0x10),
                DecodeEvent::Junk(0x00),
                DecodeEvent::Frame(b"after".to_vec()),
            ]
        );
    }

    #[test]
    fn truncated_frame_waits_for_more_data() {
        let framed = encode_frame(b"abcdef");
        let mut d = FrameDecoder::new();
        let mut out = Vec::new();
        d.feed(&framed[..6], &mut out);
        assert!(out.is_empty());
        d.feed(&framed[6..], &mut out);
        assert_eq!(out, vec![DecodeEvent::Frame(b"abcdef".to_vec())]);
    }

    #[test]
    fn length_bytes_may_look_like_markers() {
        // A valid length of 0x0094 = 148 bytes works even though the low
        // length byte equals the first start marker byte.
        let payload = vec![0x11; 0x0094];
        let framed = encode_frame(&payload);
        let events = collect(&framed);
        assert_eq!(events, vec![DecodeEvent::Frame(payload)]);
    }

    #[test]
    fn protobuf_round_trip() {
        use meshtastic_protobufs::meshtastic::{FromRadio, from_radio};
        let msg = FromRadio {
            id: 7,
            payload_variant: Some(from_radio::PayloadVariant::Rebooted(true)),
        };
        let bytes = encode_protobuf(&msg);
        let decoded = decode_from_radio(&bytes).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn encoder_wraps_toradio() {
        use meshtastic_protobufs::meshtastic::{ToRadio, to_radio};
        use prost::Message as _;
        let msg = ToRadio {
            payload_variant: Some(to_radio::PayloadVariant::WantConfigId(69_420)),
        };
        let framed = FrameEncoder::default().encode_to_radio(&msg);
        assert_eq!(&framed[..2], &[START1, START2]);
        let len = ((framed[2] as usize) << 8) | framed[3] as usize;
        assert_eq!(len, framed.len() - 4);

        // And it decodes back through the decoder.
        let events = collect(&framed);
        let DecodeEvent::Frame(payload) = events.into_iter().next().unwrap() else {
            panic!("expected a frame");
        };
        let back = ToRadio::decode(payload.as_slice()).unwrap();
        assert_eq!(back, msg);
    }
}
