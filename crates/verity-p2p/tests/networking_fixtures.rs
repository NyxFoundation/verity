//! Conformance against leanSpec networking vectors.
//!
//! Two fixture formats are consumed:
//!
//! - `networking_codec_test` (`fixtures/consensus/networking_codec/`) — wire-format
//!   vectors: varints, both snappy formats, gossip topics, the message-ID function, and
//!   req/resp chunk framing, plus rejection vectors for each decoder.
//! - `ssz_test` for `test_networking_containers` — SSZ vectors for the envelope types
//!   this crate defines (`Status`, `BlocksByRootRequest`).
//!
//! Gated on `VERITY_FIXTURES` pointing at an extracted `fixtures-prod-scheme` tree; the
//! fast `cargo test` gate leaves it unset and the tests return. CI's fixtures job always
//! sets it, and fails if no vector matched.
//!
//! Vectors for layers Verity does not hand-implement are counted and skipped, not
//! silently dropped: the gossipsub RPC protobuf lives inside upstream libp2p, and ENR /
//! peer-ID derivation belongs to a discovery layer the crate deliberately does not have.
//!
//! # What "matching" means where compression is involved
//!
//! Snappy is a self-describing format whose *compressor* output is implementation-defined:
//! the reference vectors were produced by C snappy, and Rust's `snap` legitimately picks
//! different (equally valid) encodings for some inputs. Interop needs mutual
//! decodability, not identical compressor choices, so vectors that embed compressed bytes
//! are checked as: (a) this crate's decoder recovers the payload from the *reference*
//! bytes — the direction that talks to other clients — and (b) this crate's own
//! encode/decode round trip preserves the payload. Everything uncompressed — varints,
//! topic strings, message IDs, response-code bytes, chunk structure — is checked
//! byte-exactly.
//!
//! Source: leanSpec `main` @ `0588c2d215a955a516378677a92db2a5666802f3`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use futures::executor::block_on;
use futures::io::Cursor;
use libssz::{SszDecode, SszEncode};
use libssz_merkle::{HashTreeRoot, Sha2Hasher};
use serde::Deserialize;
use verity_p2p::wire::snappy::{compress_block, compress_framed, decompress_block, read_framed};
use verity_p2p::wire::varint::{decode_varint, write_varint};
use verity_p2p::{
    BlocksByRootRequest, GossipKind, GossipTopic, MAX_PAYLOAD_SIZE, Status, message_id_with_domain,
};
use verity_types::SubnetId;

/// Codec kinds this crate implements no counterpart for. `gossipsub_rpc` is upstream
/// libp2p's protobuf; `enr` and `peer_id` are discovery-layer machinery.
const SKIPPED_KINDS: &[&str] = &["gossipsub_rpc", "enr", "peer_id"];

/// Rejection decoders skipped for the same reason.
const SKIPPED_DECODERS: &[&str] = &["gossipsub_rpc", "enr"];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodecFixture {
    codec: serde_json::Value,
    output: serde_json::Value,
    #[serde(default)]
    rejection_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SszFixture {
    type_name: String,
    serialized: String,
    #[serde(default)]
    root: String,
    #[serde(default)]
    rejection_reason: Option<String>,
}

enum Outcome {
    Matched,
    Skipped,
}

#[test]
fn should_match_leanspec_networking_codec_vectors_when_fixtures_are_present() {
    let Some(root) = fixtures_dir() else {
        eprintln!("skipping: set VERITY_FIXTURES to run leanSpec networking vectors");
        return;
    };
    let mut files = Vec::new();
    collect_json_under(&root, "networking_codec", &mut files);
    assert!(
        !files.is_empty(),
        "no networking_codec JSON under {}",
        root.display()
    );

    let mut matched = 0usize;
    let mut skipped = 0usize;
    let mut failures = Vec::new();
    for path in &files {
        for (id, fixture) in read_cases::<CodecFixture>(path, &mut failures) {
            match run_codec_case(&fixture) {
                Ok(Outcome::Matched) => matched += 1,
                Ok(Outcome::Skipped) => skipped += 1,
                Err(error) => failures.push(format!("{} ({id}): {error}", path.display())),
            }
        }
    }

    eprintln!("matched {matched} leanSpec networking codec vectors, skipped {skipped}");
    assert!(
        failures.is_empty(),
        "{} networking vector(s) disagreed with leanSpec:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(
        matched > 0,
        "no networking vector matched; {skipped} skipped"
    );
}

#[test]
fn should_match_leanspec_networking_container_vectors_when_fixtures_are_present() {
    let Some(root) = fixtures_dir() else {
        eprintln!("skipping: set VERITY_FIXTURES to run leanSpec networking vectors");
        return;
    };
    let mut files = Vec::new();
    collect_json_under(&root, "test_networking_containers", &mut files);
    assert!(
        !files.is_empty(),
        "no test_networking_containers JSON under {}",
        root.display()
    );

    let mut matched = 0usize;
    let mut failures = Vec::new();
    for path in &files {
        for (id, fixture) in read_cases::<SszFixture>(path, &mut failures) {
            match run_ssz_case(&fixture) {
                Ok(()) => matched += 1,
                Err(error) => failures.push(format!("{} ({id}): {error}", path.display())),
            }
        }
    }

    eprintln!("matched {matched} leanSpec networking container vectors");
    assert!(
        failures.is_empty(),
        "{} container vector(s) disagreed with leanSpec:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(matched > 0, "no networking container vector matched");
}

fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("VERITY_FIXTURES").map(PathBuf::from)
}

/// Collects every `.json` whose path contains `component` as a directory name.
fn collect_json_under(dir: &Path, component: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_under(&path, component, out);
            continue;
        }
        let is_json = path.extension().is_some_and(|ext| ext == "json");
        let in_suite = path.components().any(|c| c.as_os_str() == component);
        if is_json && in_suite {
            out.push(path);
        }
    }
}

fn read_cases<T: for<'de> Deserialize<'de>>(
    path: &Path,
    failures: &mut Vec<String>,
) -> BTreeMap<String, T> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            failures.push(format!("{}: read error: {error}", path.display()));
            return BTreeMap::new();
        }
    };
    match serde_json::from_str(&text) {
        Ok(cases) => cases,
        Err(error) => {
            failures.push(format!("{}: json: {error}", path.display()));
            BTreeMap::new()
        }
    }
}

// ── networking_codec dispatch ──────────────────────────────────────────────────────────

fn run_codec_case(fixture: &CodecFixture) -> Result<Outcome, String> {
    let kind = fixture.codec["kind"]
        .as_str()
        .ok_or("codec carries no kind")?;
    if SKIPPED_KINDS.contains(&kind) {
        return Ok(Outcome::Skipped);
    }
    match kind {
        "varint" => check_varint(fixture),
        "gossip_topic" => check_gossip_topic(fixture),
        "gossip_message_id" => check_message_id(fixture),
        "reqresp_request" => check_reqresp_request(fixture),
        "reqresp_response" => check_reqresp_response(fixture),
        "reqresp_response_stream" => check_response_stream(fixture),
        "snappy_block" => check_snappy_block(fixture),
        "snappy_frame" => check_snappy_frame(fixture),
        "decode_failure" => check_decode_failure(fixture),
        other => Err(format!("unhandled codec kind {other}")),
    }
}

fn field<'a>(value: &'a serde_json::Value, name: &str) -> Result<&'a serde_json::Value, String> {
    value
        .get(name)
        .ok_or_else(|| format!("missing field {name}"))
}

fn hex_field(value: &serde_json::Value, name: &str) -> Result<Vec<u8>, String> {
    from_hex(
        field(value, name)?
            .as_str()
            .ok_or(format!("{name}: not a string"))?,
    )
}

fn str_field<'a>(value: &'a serde_json::Value, name: &str) -> Result<&'a str, String> {
    field(value, name)?
        .as_str()
        .ok_or(format!("{name}: not a string"))
}

fn u64_field(value: &serde_json::Value, name: &str) -> Result<u64, String> {
    field(value, name)?
        .as_u64()
        .ok_or(format!("{name}: not a u64"))
}

fn check_varint(fixture: &CodecFixture) -> Result<Outcome, String> {
    let value = u64_field(&fixture.codec, "value")?;
    let expected = hex_field(&fixture.output, "encoded")?;
    let expected_len = u64_field(&fixture.output, "byteLength")? as usize;

    let mut encoded = Vec::new();
    write_varint(&mut encoded, value);
    if encoded != expected {
        return Err(format!(
            "varint({value}) = 0x{}, want 0x{}",
            hex_encode(&encoded),
            hex_encode(&expected)
        ));
    }
    if encoded.len() != expected_len {
        return Err(format!("byte length {} != {expected_len}", encoded.len()));
    }
    let (decoded, consumed) = decode_varint(&encoded).map_err(|e| e.to_string())?;
    if decoded != value || consumed != encoded.len() {
        return Err(format!(
            "decode({value}) round trip failed: {decoded}/{consumed}"
        ));
    }
    Ok(Outcome::Matched)
}

fn check_gossip_topic(fixture: &CodecFixture) -> Result<Outcome, String> {
    let topic_kind = str_field(&fixture.codec, "topicKind")?;
    let network_name = str_field(&fixture.codec, "networkName")?;
    let kind = match topic_kind {
        "block" => GossipKind::Block,
        "aggregation" => GossipKind::Aggregation,
        "attestation" => GossipKind::Attestation(SubnetId(u64_field(&fixture.codec, "subnetId")?)),
        other => return Err(format!("unhandled topic kind {other}")),
    };
    let topic = GossipTopic::new(kind, network_name);
    let expected = str_field(&fixture.output, "topicString")?;
    if topic.to_string() != expected {
        return Err(format!("topic {} != {expected}", topic));
    }
    let parsed = GossipTopic::parse(expected).ok_or("reference topic did not parse")?;
    if parsed != topic {
        return Err("topic parse round trip failed".into());
    }
    if let Some(expected_network) = fixture.codec.get("expectedNetworkName") {
        let expected_network = expected_network.as_str().ok_or("expectedNetworkName")?;
        let fork_valid = fixture.output["forkValid"]
            .as_bool()
            .ok_or("missing forkValid")?;
        if (parsed.network_name == expected_network) != fork_valid {
            return Err(format!(
                "fork validation of {} against {expected_network}: want {fork_valid}",
                parsed.network_name
            ));
        }
    }
    Ok(Outcome::Matched)
}

fn check_message_id(fixture: &CodecFixture) -> Result<Outcome, String> {
    let topic = hex_field(&fixture.codec, "topic")?;
    let data = hex_field(&fixture.codec, "data")?;
    let domain = hex_field(&fixture.codec, "domain")?;
    let expected = hex_field(&fixture.output, "messageId")?;
    let id = message_id_with_domain(&domain, &topic, &data);
    if id.as_slice() != expected {
        return Err(format!(
            "message id 0x{} != 0x{}",
            hex_encode(&id),
            hex_encode(&expected)
        ));
    }
    Ok(Outcome::Matched)
}

/// Encodes one req/resp chunk payload the way the crate's codec does.
fn encode_chunk(payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut wire = Vec::new();
    write_varint(&mut wire, payload.len() as u64);
    wire.extend_from_slice(&compress_framed(payload).map_err(|e| e.to_string())?);
    Ok(wire)
}

/// Decodes one whole-buffer req/resp chunk payload strictly: the varint, the payload
/// cap, the framed bytes, full consumption, and an exact length match.
fn decode_chunk(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let (declared, consumed) = decode_varint(bytes).map_err(|e| e.to_string())?;
    if declared > MAX_PAYLOAD_SIZE as u64 {
        return Err("declared length above the payload cap".into());
    }
    let mut cursor = Cursor::new(&bytes[consumed..]);
    let payload =
        block_on(read_framed(&mut cursor, declared as usize)).map_err(|e| e.to_string())?;
    if (cursor.position() as usize) != bytes.len() - consumed {
        return Err("trailing bytes after the framed payload".into());
    }
    Ok(payload)
}

fn check_reqresp_request(fixture: &CodecFixture) -> Result<Outcome, String> {
    let ssz_data = hex_field(&fixture.codec, "sszData")?;
    let expected = hex_field(&fixture.output, "encoded")?;
    if decode_chunk(&expected)? != ssz_data {
        return Err("request decode did not recover the reference payload".into());
    }
    let encoded = encode_chunk(&ssz_data)?;
    if decode_chunk(&encoded)? != ssz_data {
        return Err("request encode/decode round trip failed".into());
    }
    Ok(Outcome::Matched)
}

fn check_reqresp_response(fixture: &CodecFixture) -> Result<Outcome, String> {
    let code = u64_field(&fixture.codec, "responseCode")? as u8;
    let ssz_data = hex_field(&fixture.codec, "sszData")?;
    let expected = hex_field(&fixture.output, "encoded")?;

    let (first, rest) = expected.split_first().ok_or("empty response")?;
    if *first != code || decode_chunk(rest)? != ssz_data {
        return Err("response decode did not recover the reference code and payload".into());
    }
    let mut encoded = vec![code];
    encoded.extend_from_slice(&encode_chunk(&ssz_data)?);
    if decode_chunk(&encoded[1..])? != ssz_data {
        return Err("response encode/decode round trip failed".into());
    }
    Ok(Outcome::Matched)
}

fn check_response_stream(fixture: &CodecFixture) -> Result<Outcome, String> {
    let chunks = field(&fixture.codec, "chunks")?
        .as_array()
        .ok_or("chunks: not an array")?;
    let expected = hex_field(&fixture.output, "encoded")?;
    let expected_count = u64_field(&fixture.output, "chunkCount")? as usize;

    let mut encoded = Vec::new();
    let mut inputs = Vec::new();
    for chunk in chunks {
        let code = u64_field(chunk, "responseCode")? as u8;
        let payload = hex_field(chunk, "sszData")?;
        encoded.push(code);
        encoded.extend_from_slice(&encode_chunk(&payload)?);
        inputs.push((code, payload));
    }

    // The reference stream and this crate's own encoding must both decode, chunk by
    // chunk exactly as the wire reader works — code byte, varint, frames, repeat until
    // end of stream — back to the same chunk sequence.
    let decoded = decode_stream(&expected)?;
    if decoded.len() != expected_count || decoded != inputs {
        return Err("stream decode did not recover the reference chunk sequence".into());
    }
    if decode_stream(&encoded)? != inputs {
        return Err("stream encode/decode round trip failed".into());
    }
    Ok(Outcome::Matched)
}

/// Decodes a whole response stream into its `(code, payload)` chunks.
fn decode_stream(bytes: &[u8]) -> Result<Vec<(u8, Vec<u8>)>, String> {
    let mut decoded = Vec::new();
    let mut remaining = bytes;
    while let Some((code, rest)) = remaining.split_first() {
        let (declared, consumed) = decode_varint(rest).map_err(|e| e.to_string())?;
        let mut cursor = Cursor::new(&rest[consumed..]);
        let payload =
            block_on(read_framed(&mut cursor, declared as usize)).map_err(|e| e.to_string())?;
        let used = consumed + cursor.position() as usize;
        decoded.push((*code, payload));
        remaining = &rest[used..];
    }
    Ok(decoded)
}

fn check_snappy_block(fixture: &CodecFixture) -> Result<Outcome, String> {
    let data = hex_field(&fixture.codec, "data")?;
    let expected = hex_field(&fixture.output, "compressed")?;
    let expected_len = u64_field(&fixture.output, "compressedLength")? as usize;
    let uncompressed_len = u64_field(&fixture.output, "uncompressedLength")? as usize;
    if data.len() != uncompressed_len {
        return Err("uncompressedLength disagrees with data".into());
    }

    if expected.len() != expected_len {
        return Err("compressedLength disagrees with the reference bytes".into());
    }
    let restored = decompress_block(&expected, MAX_PAYLOAD_SIZE).map_err(|e| e.to_string())?;
    if restored != data {
        return Err("block decompression did not recover the reference payload".into());
    }
    let round_trip =
        decompress_block(&compress_block(&data), MAX_PAYLOAD_SIZE).map_err(|e| e.to_string())?;
    if round_trip != data {
        return Err("block compress/decompress round trip failed".into());
    }
    Ok(Outcome::Matched)
}

fn check_snappy_frame(fixture: &CodecFixture) -> Result<Outcome, String> {
    let data = hex_field(&fixture.codec, "data")?;
    let expected = hex_field(&fixture.output, "framed")?;
    let expected_len = u64_field(&fixture.output, "framedLength")? as usize;
    let uncompressed_len = u64_field(&fixture.output, "uncompressedLength")? as usize;

    if expected.len() != expected_len {
        return Err("framedLength disagrees with the reference bytes".into());
    }
    let mut cursor = Cursor::new(expected.as_slice());
    let restored =
        block_on(read_framed(&mut cursor, uncompressed_len)).map_err(|e| e.to_string())?;
    if restored != data || (cursor.position() as usize) != expected.len() {
        return Err("frame decompression did not recover the reference payload".into());
    }
    let framed = compress_framed(&data).map_err(|e| e.to_string())?;
    let mut cursor = Cursor::new(framed.as_slice());
    let round_trip =
        block_on(read_framed(&mut cursor, uncompressed_len)).map_err(|e| e.to_string())?;
    if round_trip != data {
        return Err("frame compress/decompress round trip failed".into());
    }
    Ok(Outcome::Matched)
}

/// Whole-buffer framed decompression with unknown length, mirroring leanSpec's
/// `frame_decompress` for rejection vectors: empty input and every malformed frame must
/// fail.
fn frame_decompress_strict(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.is_empty() {
        return Err("empty framed input".into());
    }
    let mut decoder = snap::read::FrameDecoder::new(bytes);
    let mut payload = Vec::new();
    decoder
        .read_to_end(&mut payload)
        .map_err(|e| e.to_string())?;
    Ok(payload)
}

fn check_decode_failure(fixture: &CodecFixture) -> Result<Outcome, String> {
    let decoder = str_field(&fixture.codec, "decoder")?;
    if SKIPPED_DECODERS.contains(&decoder) {
        return Ok(Outcome::Skipped);
    }
    if fixture.rejection_reason.is_none() {
        return Err("decode_failure vector carries no rejectionReason".into());
    }
    let raw = hex_field(&fixture.codec, "rawBytes")?;
    let rejected = match decoder {
        "varint" => decode_varint(&raw).is_err(),
        "snappy_block" => decompress_block(&raw, MAX_PAYLOAD_SIZE).is_err(),
        "snappy_frame" => frame_decompress_strict(&raw).is_err(),
        "reqresp_request" => decode_chunk(&raw).is_err(),
        "reqresp_response" => raw
            .split_first()
            .ok_or(())
            .and_then(|(_, rest)| decode_chunk(rest).map_err(|_| ()))
            .is_err(),
        other => return Err(format!("unhandled rejection decoder {other}")),
    };
    if rejected {
        Ok(Outcome::Matched)
    } else {
        Err(format!("{decoder} accepted input leanSpec rejects"))
    }
}

// ── SSZ container dispatch ─────────────────────────────────────────────────────────────

fn run_ssz_case(fixture: &SszFixture) -> Result<(), String> {
    let bytes = from_hex(&fixture.serialized)?;
    let reject = fixture.rejection_reason.is_some();
    let root = if reject {
        Vec::new()
    } else if fixture.root.is_empty() {
        return Err("valid vector carries no root".into());
    } else {
        from_hex(&fixture.root)?
    };
    match fixture.type_name.as_str() {
        "Status" => apply::<Status>(&bytes, &root, reject),
        "BlocksByRootRequest" => apply::<BlocksByRootRequest>(&bytes, &root, reject),
        other => Err(format!("unhandled SSZ type {other}")),
    }
}

fn apply<T>(bytes: &[u8], root: &[u8], reject: bool) -> Result<(), String>
where
    T: SszDecode + SszEncode + HashTreeRoot + PartialEq + std::fmt::Debug,
{
    if reject {
        return match T::from_ssz_bytes(bytes) {
            Err(_) => Ok(()),
            Ok(_) => Err("decode succeeded; leanSpec expects rejection".into()),
        };
    }
    let decoded = T::from_ssz_bytes(bytes).map_err(|error| format!("decode: {error:?}"))?;
    if decoded.to_ssz() != bytes {
        return Err("encode mismatch".into());
    }
    let computed = decoded.hash_tree_root(&Sha2Hasher);
    if computed.as_slice() != root {
        return Err(format!(
            "root mismatch: got 0x{}, want 0x{}",
            hex_encode(computed.as_slice()),
            hex_encode(root)
        ));
    }
    Ok(())
}

// ── hex helpers ────────────────────────────────────────────────────────────────────────

fn from_hex(text: &str) -> Result<Vec<u8>, String> {
    let text = text.strip_prefix("0x").unwrap_or(text);
    if !text.len().is_multiple_of(2) {
        return Err(format!("odd-length hex ({})", text.len()));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| format!("invalid hex at {i}")))
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
