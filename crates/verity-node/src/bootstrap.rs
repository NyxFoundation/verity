//! The peer-facing inputs a lean node is started from: who to dial, and who to be.
//!
//! Transcribed from leanSpec `src/lean_spec/cli/bootstrap.py` and
//! `src/lean_spec/node/networking/enr/enr.py`, read at commit `0b7d33ec`.
//!
//! # Bootnodes
//!
//! lean-quickstart writes every node's ENR into `nodes.yaml` so that no client has to run a
//! discovery protocol; the file is a YAML list of strings. leanSpec's `--bootnode` accepts
//! either an ENR (`enr:` followed by unpadded base64url) or a bare multiaddr, and so does
//! [`parse_bootnode`], for both the file and the command line.
//!
//! An ENR is reduced to the one address the lean transport can use, `/ip4/<ip>/udp/<port>/quic-v1`,
//! with the `quic` field preferred and `udp` the fallback, exactly as leanSpec's
//! `ENR.multiaddr()` does. Verity appends `/p2p/<peer id>`, which leanSpec does not: the peer
//! id is derived from the record's own secp256k1 key, so a dial that reaches a different
//! identity at that address fails instead of admitting a stranger as the bootnode. Every lean
//! client derives its libp2p identity from the same key that signs its record, which is what
//! makes the two agree.
//!
//! A record whose signature does not verify is refused. The list is static and hand-written,
//! so the check catches a copy-paste error rather than an attack — but a record that fails it
//! is not the record the operator meant, and dialling it would only hide that.
//!
//! # Node key
//!
//! `<node>.key` is the node's secp256k1 secret, 32 bytes of hex on one line, `0x` optional.
//! lean-quickstart writes it from the `privkey` field of `validator-config.yaml`, and the ENR
//! in `nodes.yaml` is signed with the same key, so loading it is what makes this node's
//! identity match the one every peer was told to expect.

use std::fs;
use std::path::Path;
use std::str::FromStr;

use verity_p2p::identity::{self, Keypair};
use verity_p2p::{Multiaddr, PeerId};
use verity_types::config::ATTESTATION_COMMITTEE_COUNT;

use crate::error::ConfigError;

/// The ENR key under which a lean node advertises its QUIC port.
const QUIC_KEY: &str = "quic";

/// A signed node record under the `v4` identity scheme, the only one lean clients use.
type Record = enr::Enr<enr::k256::ecdsa::SigningKey>;

/// Reads a bootnode list: a YAML sequence of ENR strings or multiaddrs.
///
/// # Errors
///
/// [`ConfigError::Unreadable`] when the file cannot be read, [`ConfigError::Malformed`] when
/// it is not a sequence of strings, and [`ConfigError::MalformedBootnode`] for the first
/// entry that is neither a valid ENR nor a multiaddr.
pub fn read_bootnodes(path: &Path) -> Result<Vec<Multiaddr>, ConfigError> {
    let text = fs::read_to_string(path).map_err(|error| ConfigError::Unreadable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    let entries: Vec<String> =
        serde_norway::from_str(&text).map_err(|error| ConfigError::Malformed {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;

    entries
        .iter()
        .map(|entry| {
            parse_bootnode(entry).map_err(|reason| ConfigError::MalformedBootnode {
                source: path.display().to_string(),
                entry: entry.clone(),
                reason,
            })
        })
        .collect()
}

/// Turns one bootnode string into a dialable address.
///
/// An `enr:` string is decoded, verified, and reduced to its QUIC multiaddr with the peer id
/// appended. Anything else must parse as a multiaddr and is passed through untouched.
///
/// # Errors
///
/// A rendered reason when the entry is neither.
pub fn parse_bootnode(entry: &str) -> Result<Multiaddr, String> {
    let entry = entry.trim();
    if entry.starts_with("enr:") {
        return multiaddr_of_enr(entry);
    }
    Multiaddr::from_str(entry).map_err(|error| format!("not a multiaddr: {error}"))
}

fn multiaddr_of_enr(text: &str) -> Result<Multiaddr, String> {
    let record = Record::from_str(text).map_err(|error| format!("invalid ENR: {error}"))?;
    if !record.verify() {
        return Err("ENR signature does not verify".to_string());
    }

    let ip = record
        .ip4()
        .ok_or_else(|| "ENR carries no IPv4 address".to_string())?;
    let port = quic_port(&record)?
        .or_else(|| record.udp4())
        .ok_or_else(|| "ENR carries neither a quic nor a udp port".to_string())?;
    let peer_id = peer_id_of(&record)?;

    let address = format!("/ip4/{ip}/udp/{port}/quic-v1/p2p/{peer_id}");
    Multiaddr::from_str(&address).map_err(|error| format!("cannot build multiaddr: {error}"))
}

/// The `quic` field, when present. Absent is `Ok(None)`; present but not a port is an error.
fn quic_port(record: &Record) -> Result<Option<u16>, String> {
    record
        .get_decodable::<u16>(QUIC_KEY)
        .transpose()
        .map_err(|error| format!("ENR quic field is not a port: {error:?}"))
}

fn peer_id_of(record: &Record) -> Result<PeerId, String> {
    use enr::EnrPublicKey as _;

    let compressed = record.public_key().encode();
    let public = identity::secp256k1::PublicKey::try_from_bytes(compressed.as_ref())
        .map_err(|error| format!("ENR public key is not a libp2p identity: {error}"))?;
    Ok(identity::PublicKey::from(public).to_peer_id())
}

/// Reads the node's secp256k1 secret and turns it into its libp2p identity.
///
/// # Errors
///
/// [`ConfigError::Unreadable`] when the file cannot be read and
/// [`ConfigError::MalformedNodeKey`] when its contents are not a 32-byte secp256k1 secret.
pub fn read_node_key(path: &Path) -> Result<Keypair, ConfigError> {
    let text = fs::read_to_string(path).map_err(|error| ConfigError::Unreadable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })?;
    keypair_from_hex(&text).map_err(|reason| ConfigError::MalformedNodeKey {
        path: path.to_path_buf(),
        reason,
    })
}

fn keypair_from_hex(text: &str) -> Result<Keypair, String> {
    let trimmed = text.trim();
    let trimmed = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    let mut bytes = hex::decode(trimmed).map_err(|error| format!("not hex: {error}"))?;
    if bytes.len() != 32 {
        return Err(format!("expected 32 bytes, found {}", bytes.len()));
    }
    let secret = identity::secp256k1::SecretKey::try_from_bytes(&mut bytes)
        .map_err(|error| format!("not a secp256k1 secret: {error}"))?;
    Ok(Keypair::from(identity::secp256k1::Keypair::from(secret)))
}

/// Refuses a committee count the chain constants do not implement.
///
/// leanSpec fixes `ATTESTATION_COMMITTEE_COUNT` as a constant of the fork, and Verity
/// transcribes it as one; the flag exists so that a lean-quickstart deployment configured for
/// another value fails at startup, with the reason, rather than joining a network whose
/// subnet layout it disagrees with.
///
/// # Errors
///
/// [`ConfigError::UnsupportedSetting`] when `count` is not the constant.
pub fn check_committee_count(count: u64) -> Result<(), ConfigError> {
    if count == ATTESTATION_COMMITTEE_COUNT {
        return Ok(());
    }
    Err(ConfigError::UnsupportedSetting {
        setting: "attestation committee count",
        reason: format!("this build implements {ATTESTATION_COMMITTEE_COUNT}, not {count}"),
    })
}

/// Refuses an aggregation subnet the chain constants do not define.
///
/// # Errors
///
/// [`ConfigError::UnsupportedSetting`] when any id is at or above the committee count.
pub fn check_aggregate_subnets(subnets: &[u64]) -> Result<(), ConfigError> {
    match subnets
        .iter()
        .find(|id| **id >= ATTESTATION_COMMITTEE_COUNT)
    {
        None => Ok(()),
        Some(id) => Err(ConfigError::UnsupportedSetting {
            setting: "aggregate subnet ids",
            reason: format!(
                "subnet {id} does not exist with {ATTESTATION_COMMITTEE_COUNT} committee(s)"
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::Ipv4Addr;

    use super::*;

    /// The first record of the `nodes.yaml` sample in lean-quickstart's README: 127.0.0.1,
    /// quic 9000, signed by a secp256k1 key.
    const README_ENR: &str = "enr:-IW4QMn2QUYENcnsEpITZLph3YZee8Y3B92INUje_riQUOFQQ5Zm5kASi7E_IuQoGCWgcmCYrH920Q52kH7tQcWcPhEBgmlkgnY0gmlwhH8AAAGEcXVpY4IjKIlzZWNwMjU2azGhAhMMnGF1rmIPQ9tWgqfkNmvsG-aIyc9EJU5JFo3Tegys";

    const SECRET_HEX: &str = "bdf953adc161873ba026330c56450453f582e3c4ee6cb713644794bcfdd85fe5";

    fn signing_key() -> enr::k256::ecdsa::SigningKey {
        let bytes = hex::decode(SECRET_HEX).expect("hex");
        enr::k256::ecdsa::SigningKey::from_slice(&bytes).expect("a secp256k1 secret")
    }

    fn write(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("a temporary file");
        file.write_all(contents.as_bytes()).expect("write");
        file.flush().expect("flush");
        file
    }

    #[test]
    fn should_reduce_the_readme_enr_to_its_quic_address() {
        let address = parse_bootnode(README_ENR).expect("the README record parses");
        let text = address.to_string();
        assert!(
            text.starts_with("/ip4/127.0.0.1/udp/9000/quic-v1/p2p/"),
            "unexpected address {text}"
        );
    }

    #[test]
    fn should_derive_the_same_peer_id_the_node_key_produces() {
        let key = signing_key();
        let record = Record::builder()
            .ip4(Ipv4Addr::LOCALHOST)
            .add_value(QUIC_KEY, &9001u16)
            .build(&key)
            .expect("a signed record");

        let address = parse_bootnode(&record.to_base64()).expect("parses");
        let keypair = keypair_from_hex(SECRET_HEX).expect("the same secret");
        let expected = format!(
            "/ip4/127.0.0.1/udp/9001/quic-v1/p2p/{}",
            keypair.public().to_peer_id()
        );
        assert_eq!(address.to_string(), expected);
    }

    #[test]
    fn should_fall_back_to_the_udp_port_when_quic_is_absent() {
        let record = Record::builder()
            .ip4(Ipv4Addr::new(10, 0, 0, 7))
            .udp4(9100)
            .build(&signing_key())
            .expect("a signed record");

        let address = parse_bootnode(&record.to_base64()).expect("parses");
        assert!(
            address
                .to_string()
                .starts_with("/ip4/10.0.0.7/udp/9100/quic-v1/p2p/")
        );
    }

    #[test]
    fn should_refuse_an_enr_without_an_address() {
        let record = Record::builder()
            .udp4(9100)
            .build(&signing_key())
            .expect("a signed record");

        let error = parse_bootnode(&record.to_base64()).expect_err("no ip");
        assert!(error.contains("IPv4"), "{error}");
    }

    #[test]
    fn should_refuse_a_tampered_enr() {
        // Flip a character inside the signed content; the signature no longer covers it.
        let mut tampered = README_ENR.to_string();
        let tail = tampered.len() - 8;
        tampered.replace_range(tail..tail + 1, "A");
        assert!(parse_bootnode(&tampered).is_err());
    }

    #[test]
    fn should_pass_a_bare_multiaddr_through() {
        let address = parse_bootnode(" /ip4/10.0.0.1/udp/9000/quic-v1 ").expect("parses");
        assert_eq!(address.to_string(), "/ip4/10.0.0.1/udp/9000/quic-v1");
    }

    #[test]
    fn should_refuse_anything_that_is_neither() {
        assert!(parse_bootnode("localhost:9000").is_err());
    }

    #[test]
    fn should_read_a_nodes_yaml_list_and_name_the_bad_entry() {
        let file = write(&format!(
            "- {README_ENR}\n- /ip4/10.0.0.1/udp/9000/quic-v1\n"
        ));
        let addresses = read_bootnodes(file.path()).expect("both entries parse");
        assert_eq!(addresses.len(), 2);

        let bad = write("- not-an-address\n");
        match read_bootnodes(bad.path()) {
            Err(ConfigError::MalformedBootnode { entry, .. }) => {
                assert_eq!(entry, "not-an-address");
            }
            other => panic!("expected MalformedBootnode, got {other:?}"),
        }
    }

    #[test]
    fn should_read_a_node_key_with_or_without_a_prefix() {
        let bare = write(&format!("{SECRET_HEX}\n"));
        let prefixed = write(&format!("0x{SECRET_HEX}"));
        let a = read_node_key(bare.path()).expect("bare hex loads");
        let b = read_node_key(prefixed.path()).expect("prefixed hex loads");
        assert_eq!(a.public().to_peer_id(), b.public().to_peer_id());
    }

    #[test]
    fn should_refuse_a_node_key_of_the_wrong_length() {
        let short = write("abcd");
        match read_node_key(short.path()) {
            Err(ConfigError::MalformedNodeKey { reason, .. }) => {
                assert!(reason.contains("32 bytes"), "{reason}");
            }
            other => panic!("expected MalformedNodeKey, got {other:?}"),
        }
    }

    #[test]
    fn should_accept_only_the_transcribed_committee_count() {
        assert!(check_committee_count(ATTESTATION_COMMITTEE_COUNT).is_ok());
        assert!(check_committee_count(ATTESTATION_COMMITTEE_COUNT + 1).is_err());
        assert!(check_aggregate_subnets(&[0]).is_ok());
        assert!(check_aggregate_subnets(&[0, ATTESTATION_COMMITTEE_COUNT]).is_err());
    }
}
