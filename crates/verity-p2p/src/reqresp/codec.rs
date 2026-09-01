//! The req/resp stream codec: varint-prefixed, snappy-framed SSZ chunks.
//!
//! Wire shapes, from leanSpec `node/networking/reqresp/codec.py`:
//!
//! ```text
//! request        := [varint: uncompressed length][snappy-framed SSZ payload]
//! response chunk := [response code: 1 byte][varint: uncompressed length][snappy-framed payload]
//! ```
//!
//! A `Status` response is exactly one chunk. A blocks response is zero or more `SUCCESS`
//! chunks — one raw SSZ `SignedBlock` each — terminated by the end of the stream, or cut
//! short by a single error chunk. Every declared length is capped before any allocation:
//! the caps are what stand between a hostile length claim and memory.

use std::io;

use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::request_response;
use libssz::{SszDecode, SszEncode};

use crate::config::{MAX_ERROR_MESSAGE_SIZE, MAX_PAYLOAD_SIZE, MAX_REQUEST_BLOCKS};
use crate::reqresp::messages::{ErrorCode, Protocol, RESPONSE_CODE_SUCCESS, Request, Response};
use crate::wire::snappy::{compress_framed, read_framed};
use crate::wire::varint::{read_varint, write_varint};

/// Largest legal SSZ encoding of each request, used as the read cap: 80 bytes of
/// `Status`, a 4-byte offset plus 32 bytes per root for `BlocksByRootRequest`, and two
/// fixed `uint64` fields for `BlocksByRangeRequest`.
const fn max_request_len(protocol: Protocol) -> usize {
    match protocol {
        Protocol::Status => 80,
        Protocol::BlocksByRoot => 4 + 32 * MAX_REQUEST_BLOCKS,
        Protocol::BlocksByRange => 16,
    }
}

/// The stream codec shared by the three per-protocol behaviours; the negotiated protocol
/// arrives as an argument and selects the envelope type.
#[derive(Debug, Clone, Default)]
pub struct Codec;

#[async_trait]
impl request_response::Codec for Codec {
    type Protocol = Protocol;
    type Request = Request;
    type Response = Response;

    async fn read_request<T>(&mut self, protocol: &Protocol, io: &mut T) -> io::Result<Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let payload = read_chunk_payload(io, max_request_len(*protocol)).await?;
        match protocol {
            Protocol::Status => Ok(Request::Status(decode_ssz(&payload)?)),
            Protocol::BlocksByRoot => Ok(Request::BlocksByRoot(decode_ssz(&payload)?)),
            Protocol::BlocksByRange => Ok(Request::BlocksByRange(decode_ssz(&payload)?)),
        }
    }

    async fn read_response<T>(&mut self, protocol: &Protocol, io: &mut T) -> io::Result<Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        match protocol {
            Protocol::Status => read_status_response(io).await,
            Protocol::BlocksByRoot | Protocol::BlocksByRange => read_blocks_response(io).await,
        }
    }

    async fn write_request<T>(
        &mut self,
        protocol: &Protocol,
        io: &mut T,
        request: Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        if request.protocol() != *protocol {
            return Err(io::Error::other("request variant does not match protocol"));
        }
        let ssz = match &request {
            Request::Status(status) => status.to_ssz(),
            Request::BlocksByRoot(roots) => roots.to_ssz(),
            Request::BlocksByRange(range) => range.to_ssz(),
        };
        io.write_all(&encode_chunk_payload(&ssz)?).await
    }

    async fn write_response<T>(
        &mut self,
        protocol: &Protocol,
        io: &mut T,
        response: Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let wire = encode_response(*protocol, &response)?;
        io.write_all(&wire).await
    }
}

/// Encodes one chunk payload: varint of the uncompressed length, then the framed bytes.
fn encode_chunk_payload(payload: &[u8]) -> io::Result<Vec<u8>> {
    let mut wire = Vec::new();
    write_varint(&mut wire, payload.len() as u64);
    wire.extend_from_slice(&compress_framed(payload)?);
    Ok(wire)
}

/// Reads one chunk payload, refusing a declared length above `max` before reading a
/// single frame.
async fn read_chunk_payload<T>(io: &mut T, max: usize) -> io::Result<Vec<u8>>
where
    T: AsyncRead + Unpin + Send,
{
    let declared = read_varint(io).await?;
    if declared > max as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chunk declares a length above the protocol cap",
        ));
    }
    // The cap fits in usize on every supported target, so the narrowing is checked once.
    let declared = usize::try_from(declared)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "length beyond addressable"))?;
    read_framed(io, declared).await
}

/// Reads the single chunk of a `Status` response.
async fn read_status_response<T>(io: &mut T) -> io::Result<Response>
where
    T: AsyncRead + Unpin + Send,
{
    let mut code = [0u8; 1];
    io.read_exact(&mut code).await?;
    match ErrorCode::from_byte(code[0]) {
        None => {
            let payload = read_chunk_payload(io, max_request_len(Protocol::Status)).await?;
            Ok(Response::Status(decode_ssz(&payload)?))
        }
        Some(code) => read_error_chunk(io, code).await,
    }
}

/// Reads a blocks response: `SUCCESS` chunks until end-of-stream, or one error chunk.
async fn read_blocks_response<T>(io: &mut T) -> io::Result<Response>
where
    T: AsyncRead + Unpin + Send,
{
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    loop {
        let mut code = [0u8; 1];
        if io.read(&mut code).await? == 0 {
            return Ok(Response::Blocks(chunks));
        }
        match ErrorCode::from_byte(code[0]) {
            None => {
                if chunks.len() == MAX_REQUEST_BLOCKS {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "more response chunks than MAX_REQUEST_BLOCKS",
                    ));
                }
                chunks.push(read_chunk_payload(io, MAX_PAYLOAD_SIZE).await?);
            }
            Some(code) => return read_error_chunk(io, code).await,
        }
    }
}

/// Reads the message chunk of an error response.
async fn read_error_chunk<T>(io: &mut T, code: ErrorCode) -> io::Result<Response>
where
    T: AsyncRead + Unpin + Send,
{
    let payload = read_chunk_payload(io, MAX_ERROR_MESSAGE_SIZE).await?;
    Ok(Response::Error {
        code,
        // Untrusted peer text destined for logs: lossy decoding cannot fail and cannot
        // grow past the already-enforced byte cap.
        message: String::from_utf8_lossy(&payload).into_owned(),
    })
}

/// Encodes a whole response for the stream, enforcing protocol/variant coherence and the
/// responder-side caps.
fn encode_response(protocol: Protocol, response: &Response) -> io::Result<Vec<u8>> {
    match (protocol, response) {
        (Protocol::Status, Response::Status(status)) => {
            let mut wire = vec![RESPONSE_CODE_SUCCESS];
            wire.extend_from_slice(&encode_chunk_payload(&status.to_ssz())?);
            Ok(wire)
        }
        (Protocol::BlocksByRoot | Protocol::BlocksByRange, Response::Blocks(chunks)) => {
            if chunks.len() > MAX_REQUEST_BLOCKS {
                return Err(io::Error::other(
                    "more block chunks than MAX_REQUEST_BLOCKS",
                ));
            }
            let mut wire = Vec::new();
            for chunk in chunks {
                if chunk.len() > MAX_PAYLOAD_SIZE {
                    return Err(io::Error::other("block chunk above the payload cap"));
                }
                wire.push(RESPONSE_CODE_SUCCESS);
                wire.extend_from_slice(&encode_chunk_payload(chunk)?);
            }
            Ok(wire)
        }
        (_, Response::Error { code, message }) => {
            if message.len() > MAX_ERROR_MESSAGE_SIZE {
                return Err(io::Error::other("error message above the message cap"));
            }
            let mut wire = vec![code.as_byte()];
            wire.extend_from_slice(&encode_chunk_payload(message.as_bytes())?);
            Ok(wire)
        }
        (Protocol::Status, Response::Blocks(_))
        | (Protocol::BlocksByRoot | Protocol::BlocksByRange, Response::Status(_)) => {
            Err(io::Error::other("response variant does not match protocol"))
        }
    }
}

/// Decodes an SSZ payload, mapping the failure onto the codec's error space.
fn decode_ssz<V: SszDecode>(payload: &[u8]) -> io::Result<V> {
    V::from_ssz_bytes(payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("ssz: {e:?}")))
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use futures::io::Cursor;
    use libssz_types::SszList;
    use request_response::Codec as _;
    use verity_types::{Checkpoint, Slot};

    use super::*;
    use crate::reqresp::messages::{BlocksByRangeRequest, BlocksByRootRequest, Status};

    fn sample_status() -> Status {
        Status {
            finalized: Checkpoint {
                root: [3u8; 32],
                slot: Slot(100),
            },
            head: Checkpoint {
                root: [4u8; 32],
                slot: Slot(140),
            },
        }
    }

    fn round_trip_request(protocol: Protocol, request: Request) -> Request {
        block_on(async {
            let mut wire = Cursor::new(Vec::new());
            Codec
                .write_request(&protocol, &mut wire, request)
                .await
                .expect("write");
            let mut wire = Cursor::new(wire.into_inner());
            Codec
                .read_request(&protocol, &mut wire)
                .await
                .expect("read")
        })
    }

    fn round_trip_response(protocol: Protocol, response: Response) -> Response {
        block_on(async {
            let mut wire = Cursor::new(Vec::new());
            Codec
                .write_response(&protocol, &mut wire, response)
                .await
                .expect("write");
            let mut wire = Cursor::new(wire.into_inner());
            Codec
                .read_response(&protocol, &mut wire)
                .await
                .expect("read")
        })
    }

    #[test]
    fn should_round_trip_every_request_kind() {
        let status = Request::Status(sample_status());
        assert_eq!(round_trip_request(Protocol::Status, status.clone()), status);

        let roots = Request::BlocksByRoot(BlocksByRootRequest {
            roots: SszList::try_from(vec![[7u8; 32], [8u8; 32]]).expect("within limit"),
        });
        assert_eq!(
            round_trip_request(Protocol::BlocksByRoot, roots.clone()),
            roots
        );

        let range = Request::BlocksByRange(BlocksByRangeRequest {
            start_slot: Slot(42),
            count: 512,
        });
        assert_eq!(
            round_trip_request(Protocol::BlocksByRange, range.clone()),
            range
        );
    }

    #[test]
    fn should_round_trip_status_and_blocks_responses() {
        let status = Response::Status(sample_status());
        assert_eq!(
            round_trip_response(Protocol::Status, status.clone()),
            status
        );

        let blocks = Response::Blocks(vec![vec![1u8; 300], vec![2u8; 4096], Vec::new()]);
        assert_eq!(
            round_trip_response(Protocol::BlocksByRange, blocks.clone()),
            blocks
        );

        let empty = Response::Blocks(Vec::new());
        assert_eq!(
            round_trip_response(Protocol::BlocksByRoot, empty.clone()),
            empty
        );
    }

    #[test]
    fn should_round_trip_error_responses_on_every_protocol() {
        for protocol in [
            Protocol::Status,
            Protocol::BlocksByRoot,
            Protocol::BlocksByRange,
        ] {
            let error = Response::Error {
                code: ErrorCode::ResourceUnavailable,
                message: "below the serving window".to_string(),
            };
            assert_eq!(round_trip_response(protocol, error.clone()), error);
        }
    }

    #[test]
    fn should_reject_write_when_variant_does_not_match_protocol() {
        block_on(async {
            let mut wire = Cursor::new(Vec::new());
            let result = Codec
                .write_request(
                    &Protocol::Status,
                    &mut wire,
                    Request::BlocksByRange(BlocksByRangeRequest::default()),
                )
                .await;
            assert!(result.is_err());

            let mut wire = Cursor::new(Vec::new());
            let result = Codec
                .write_response(&Protocol::Status, &mut wire, Response::Blocks(Vec::new()))
                .await;
            assert!(result.is_err());
        });
    }

    #[test]
    fn should_reject_request_when_declared_length_exceeds_protocol_cap() {
        block_on(async {
            // A Status request claiming 81 bytes: over the 80-byte cap, refused before
            // any frame is read.
            let mut wire = Vec::new();
            write_varint(&mut wire, 81);
            let mut wire = Cursor::new(wire);
            let err = Codec
                .read_request(&Protocol::Status, &mut wire)
                .await
                .expect_err("over cap");
            assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        });
    }

    #[test]
    fn should_degrade_unknown_error_codes_when_reading_responses() {
        block_on(async {
            let mut wire = vec![0x7f]; // unknown code in the server-error range
            wire.extend_from_slice(&encode_chunk_payload(b"boom").expect("encode"));
            let mut wire = Cursor::new(wire);
            let response = Codec
                .read_response(&Protocol::Status, &mut wire)
                .await
                .expect("read");
            assert_eq!(
                response,
                Response::Error {
                    code: ErrorCode::ServerError,
                    message: "boom".to_string(),
                }
            );
        });
    }

    #[test]
    fn should_stop_at_error_chunk_when_blocks_stream_fails_midway() {
        block_on(async {
            let mut wire = vec![RESPONSE_CODE_SUCCESS];
            wire.extend_from_slice(&encode_chunk_payload(&[9u8; 64]).expect("encode"));
            wire.push(ErrorCode::ServerError.as_byte());
            wire.extend_from_slice(&encode_chunk_payload(b"lost the database").expect("encode"));
            let mut wire = Cursor::new(wire);
            let response = Codec
                .read_response(&Protocol::BlocksByRange, &mut wire)
                .await
                .expect("read");
            assert_eq!(
                response,
                Response::Error {
                    code: ErrorCode::ServerError,
                    message: "lost the database".to_string(),
                }
            );
        });
    }
}
