//! Snappy, in the two formats the lean wire protocol uses.
//!
//! Gossip payloads use the **raw block** format. Req/resp chunks use the **framed** format
//! (CRC-checked frames per google/snappy's `framing_format.txt`). The two are not
//! interchangeable: a raw block has no frame headers, and a framed stream is unreadable as
//! a raw block. leanSpec `node/networking/client/event_source/live.py` states the split;
//! this module keeps both behind names that say which is which.
//!
//! The framed reader works frame by frame rather than wrapping the stream in a
//! decompressor, because a req/resp response is a *sequence* of chunks on one stream: the
//! reader must consume exactly the frames carrying the declared uncompressed length and
//! leave the next chunk's first byte in place. Frame accounting is the only way to know
//! where to stop without buffering the whole stream.

use std::io::{self, Read, Write};

use futures::{AsyncRead, AsyncReadExt};

use crate::config::max_compressed_len;

/// Frame-type byte for the stream identifier ("sNaPpY").
const FRAME_STREAM_IDENTIFIER: u8 = 0xff;
/// Frame-type byte for a compressed data frame.
const FRAME_COMPRESSED_DATA: u8 = 0x00;
/// Frame-type byte for an uncompressed data frame.
const FRAME_UNCOMPRESSED_DATA: u8 = 0x01;
/// First frame-type byte of the reserved unskippable range (through 0x7f).
const FRAME_RESERVED_UNSKIPPABLE_MIN: u8 = 0x02;
/// Last frame-type byte of the reserved unskippable range.
const FRAME_RESERVED_UNSKIPPABLE_MAX: u8 = 0x7f;
/// Bytes of CRC-32C prefixing the data in every data frame.
const FRAME_CRC_LEN: usize = 4;

/// Compresses a gossip payload in the raw block format.
#[must_use]
pub fn compress_block(payload: &[u8]) -> Vec<u8> {
    snap::raw::Encoder::new()
        .compress_vec(payload)
        // Raw-block compression of an in-memory slice has no failure mode: every input
        // is compressible and the output buffer is sized by the encoder itself.
        .unwrap_or_default()
}

/// Decompresses a raw-block gossip payload, refusing any declared length above `max`.
pub fn decompress_block(compressed: &[u8], max: usize) -> io::Result<Vec<u8>> {
    let declared = snap::raw::decompress_len(compressed)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if declared > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "snappy block declares a length above the payload cap",
        ));
    }
    snap::raw::Decoder::new()
        .decompress_vec(compressed)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// The complete stream-identifier frame every framed section begins with.
const STREAM_IDENTIFIER_FRAME: [u8; 10] =
    [0xff, 0x06, 0x00, 0x00, b's', b'N', b'a', b'P', b'p', b'Y'];

/// Compresses a req/resp payload in the framed format, stream identifier included.
///
/// An empty payload still carries the identifier — leanSpec's `frame_compress(b"")` is
/// the ten identifier bytes, and a reader consumes them to find the next chunk's first
/// byte. `snap`'s encoder writes its header lazily on the first data byte, so the empty
/// case is spelled out.
pub fn compress_framed(payload: &[u8]) -> io::Result<Vec<u8>> {
    if payload.is_empty() {
        return Ok(STREAM_IDENTIFIER_FRAME.to_vec());
    }
    let mut encoder = snap::write::FrameEncoder::new(Vec::new());
    encoder.write_all(payload)?;
    encoder
        .into_inner()
        .map_err(|e| io::Error::other(e.to_string()))
}

/// Reads exactly the snappy frames carrying `uncompressed_len` bytes off an async stream
/// and returns the decompressed payload, leaving the stream positioned on the byte after
/// the last consumed frame.
///
/// Refuses reserved unskippable frames, a malformed stream identifier, a frame sequence
/// whose decompressed size oversteps the declared length, and a compressed byte count
/// beyond [`max_compressed_len`] of the declared length. CRC verification happens in the
/// final decode pass over the collected frames.
pub async fn read_framed<T>(io: &mut T, uncompressed_len: usize) -> io::Result<Vec<u8>>
where
    T: AsyncRead + Unpin + Send,
{
    let max_compressed = max_compressed_len(uncompressed_len);
    let mut collected: Vec<u8> = Vec::new();
    let mut decompressed_total = 0usize;
    // Every framed section begins with the stream identifier — even a zero-length one,
    // which is *only* the identifier. Reading at least one frame here is what keeps a
    // multi-chunk stream aligned: the identifier's ten bytes must not be mistaken for
    // the next chunk's response code.
    let mut first = true;
    while first || decompressed_total < uncompressed_len {
        let mut header = [0u8; 4];
        io.read_exact(&mut header).await?;
        let frame_type = header[0];
        let payload_len =
            usize::from(header[1]) | usize::from(header[2]) << 8 | usize::from(header[3]) << 16;
        if first && frame_type != FRAME_STREAM_IDENTIFIER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "framed section does not start with the stream identifier",
            ));
        }
        first = false;
        if collected.len() + header.len() + payload_len > max_compressed {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "snappy frames exceed the worst-case compressed size",
            ));
        }
        let mut payload = vec![0u8; payload_len];
        io.read_exact(&mut payload).await?;
        decompressed_total += frame_contribution(frame_type, &payload)?;
        if decompressed_total > uncompressed_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "snappy frames decompress past the declared length",
            ));
        }
        collected.extend_from_slice(&header);
        collected.extend_from_slice(&payload);
    }
    decode_collected_frames(&collected, uncompressed_len)
}

/// How many decompressed bytes a frame contributes, validating the frame type.
fn frame_contribution(frame_type: u8, payload: &[u8]) -> io::Result<usize> {
    match frame_type {
        FRAME_STREAM_IDENTIFIER => {
            if payload == b"sNaPpY" {
                Ok(0)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "malformed snappy stream identifier",
                ))
            }
        }
        FRAME_COMPRESSED_DATA => {
            let data = payload.get(FRAME_CRC_LEN..).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "snappy data frame too short")
            })?;
            snap::raw::decompress_len(data)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }
        FRAME_UNCOMPRESSED_DATA => {
            if payload.len() < FRAME_CRC_LEN {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "snappy data frame too short",
                ));
            }
            Ok(payload.len() - FRAME_CRC_LEN)
        }
        FRAME_RESERVED_UNSKIPPABLE_MIN..=FRAME_RESERVED_UNSKIPPABLE_MAX => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "reserved unskippable snappy frame",
        )),
        // Padding (0xfe) and the reserved skippable range (0x80–0xfd) carry no data.
        _ => Ok(0),
    }
}

/// Decompresses the collected frames, verifying CRCs and the exact declared length.
fn decode_collected_frames(collected: &[u8], uncompressed_len: usize) -> io::Result<Vec<u8>> {
    let mut decoder = snap::read::FrameDecoder::new(collected);
    let mut payload = Vec::with_capacity(uncompressed_len);
    decoder.read_to_end(&mut payload)?;
    if payload.len() != uncompressed_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "snappy frames decompress to a different length than declared",
        ));
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use futures::io::Cursor;

    use super::*;

    #[test]
    fn should_round_trip_when_block_compressed() {
        let payload = b"lean consensus".repeat(100);
        let compressed = compress_block(&payload);
        let restored = decompress_block(&compressed, payload.len()).expect("round trip");
        assert_eq!(restored, payload);
    }

    #[test]
    fn should_reject_block_when_declared_length_exceeds_cap() {
        let compressed = compress_block(&[0u8; 1024]);
        assert!(decompress_block(&compressed, 1023).is_err());
    }

    #[test]
    fn should_round_trip_when_frame_compressed() {
        let payload = b"status and blocks".repeat(9000);
        let compressed = compress_framed(&payload).expect("compress");
        let restored =
            block_on(read_framed(&mut Cursor::new(compressed), payload.len())).expect("read");
        assert_eq!(restored, payload);
    }

    #[test]
    fn should_leave_following_bytes_unread_when_chunk_ends() {
        let payload = vec![7u8; 500];
        let mut wire = compress_framed(&payload).expect("compress");
        wire.extend_from_slice(b"NEXT");
        let mut cursor = Cursor::new(wire);
        let restored = block_on(read_framed(&mut cursor, payload.len())).expect("read");
        assert_eq!(restored, payload);
        let mut rest = Vec::new();
        block_on(cursor.read_to_end(&mut rest)).expect("rest");
        assert_eq!(rest, b"NEXT");
    }

    #[test]
    fn should_carry_only_the_stream_identifier_when_payload_is_empty() {
        let compressed = compress_framed(&[]).expect("compress");
        assert_eq!(compressed, STREAM_IDENTIFIER_FRAME);

        let mut wire = compressed;
        wire.extend_from_slice(b"XX");
        let mut cursor = Cursor::new(wire);
        let restored = block_on(read_framed(&mut cursor, 0)).expect("read");
        assert!(restored.is_empty());
        let mut rest = Vec::new();
        block_on(cursor.read_to_end(&mut rest)).expect("rest");
        assert_eq!(rest, b"XX");
    }

    #[test]
    fn should_reject_frames_when_reserved_unskippable_type_appears() {
        // Stream identifier, then a reserved unskippable frame.
        let mut wire = vec![0xff, 0x06, 0x00, 0x00];
        wire.extend_from_slice(b"sNaPpY");
        wire.extend_from_slice(&[0x02, 0x01, 0x00, 0x00, 0xaa]);
        let err = block_on(read_framed(&mut Cursor::new(wire), 1)).expect_err("reserved");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn should_reject_frames_when_they_decompress_past_declared_length() {
        let compressed = compress_framed(&[1u8; 100]).expect("compress");
        let err = block_on(read_framed(&mut Cursor::new(compressed), 50)).expect_err("overrun");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn should_reject_frames_when_crc_is_corrupted() {
        let payload = vec![3u8; 200];
        let mut wire = compress_framed(&payload).expect("compress");
        // Flip a CRC bit in the first data frame (after the 10-byte stream identifier
        // and the 4-byte frame header).
        wire[14] ^= 0xff;
        assert!(block_on(read_framed(&mut Cursor::new(wire), payload.len())).is_err());
    }
}
