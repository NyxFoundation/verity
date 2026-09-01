//! Unsigned LEB128 varints, as leanSpec `node/networking/varint.py` defines them.
//!
//! A `u64` occupies at most [`MAX_VARINT_LEN`] bytes; a tenth byte may only contribute the
//! single remaining bit. Reads are strict about both bounds because a varint prefixes every
//! req/resp chunk: a decoder lenient here would let a peer smuggle an unbounded length claim
//! past the payload caps.

use std::io;

use futures::{AsyncRead, AsyncReadExt};

/// Maximum encoded length of a `u64` varint.
pub const MAX_VARINT_LEN: usize = 10;

/// Appends the LEB128 encoding of `value` to `out`.
pub fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Reads one varint from an async stream, one byte at a time.
///
/// Errors on end-of-stream mid-varint, on an encoding longer than [`MAX_VARINT_LEN`]
/// bytes, and on a tenth byte carrying more than the one bit a `u64` has left.
pub async fn read_varint<T>(io: &mut T) -> io::Result<u64>
where
    T: AsyncRead + Unpin + Send,
{
    let mut value: u64 = 0;
    for index in 0..MAX_VARINT_LEN {
        let mut byte = [0u8; 1];
        io.read_exact(&mut byte).await?;
        let group = u64::from(byte[0] & 0x7f);
        // The tenth byte holds bits 63.. — anything beyond bit 63 overflows a u64.
        if index == MAX_VARINT_LEN - 1 && group > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "varint overflows u64",
            ));
        }
        value |= group << (7 * index);
        if byte[0] & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "varint longer than 10 bytes",
    ))
}

/// Decodes one varint from the front of a byte slice, returning the value and the number
/// of bytes consumed. Same strictness as [`read_varint`].
pub fn decode_varint(bytes: &[u8]) -> io::Result<(u64, usize)> {
    let mut value: u64 = 0;
    for (index, byte) in bytes.iter().enumerate().take(MAX_VARINT_LEN) {
        let group = u64::from(byte & 0x7f);
        if index == MAX_VARINT_LEN - 1 && group > 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "varint overflows u64",
            ));
        }
        value |= group << (7 * index);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    let kind = if bytes.len() < MAX_VARINT_LEN {
        io::ErrorKind::UnexpectedEof
    } else {
        io::ErrorKind::InvalidData
    };
    Err(io::Error::new(kind, "truncated or over-long varint"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_round_trip_boundary_values_when_encoded_and_decoded() {
        for value in [0u64, 1, 127, 128, 300, 16383, 16384, u64::MAX - 1, u64::MAX] {
            let mut buf = Vec::new();
            write_varint(&mut buf, value);
            let (decoded, consumed) = decode_varint(&buf).expect("round trip");
            assert_eq!(decoded, value);
            assert_eq!(consumed, buf.len());
        }
    }

    #[test]
    fn should_reject_encoding_when_it_overflows_u64() {
        // Ten continuation groups followed by a value the tenth byte cannot hold.
        let bytes = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02];
        assert!(decode_varint(&bytes).is_err());
    }

    #[test]
    fn should_reject_encoding_when_longer_than_ten_bytes() {
        let bytes = [0x80u8; 11];
        assert!(decode_varint(&bytes).is_err());
    }

    #[test]
    fn should_report_eof_when_the_final_byte_is_missing() {
        let err = decode_varint(&[0x80]).expect_err("truncated");
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
