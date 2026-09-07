//! What the two end-to-end node tests both need: production-scheme keys, and the
//! cross-client configuration files a node starts from.
//!
//! Both harnesses are gated on `VERITY_TEST_KEYS` and both write lean-quickstart's layout,
//! so the loading and the file writing live here rather than in each test. Every integration
//! test binary compiles this module separately, hence the blanket `dead_code` allowance.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use verity_crypto::SecretKey;
use verity_crypto::containers::PublicKey;

/// A key pair as `leansig-test-keys` ships it.
pub struct TestKey {
    pub public: PublicKey,
    pub secret: SecretKey,
}

/// Loads the first two key files in `VERITY_TEST_KEYS`, or `None` when the gate is off.
pub fn test_keys() -> Option<Vec<TestKey>> {
    let directory = PathBuf::from(std::env::var_os("VERITY_TEST_KEYS")?);
    assert!(
        directory.is_dir(),
        "VERITY_TEST_KEYS is set but {} is not a directory",
        directory.display()
    );

    let mut files: Vec<PathBuf> = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    assert!(
        files.len() >= 2,
        "{} holds {} key files, needed 2",
        directory.display(),
        files.len()
    );

    Some(files.iter().take(2).map(read_key).collect())
}

pub fn read_key(path: &PathBuf) -> TestKey {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

    // The file is `{"attestation_keypair": {"public_key": "0x..", "secret_key": "0x.."}}`;
    // reaching into it by hand keeps this test from taking a JSON dependency for two fields.
    let public = from_hex(field(&text, "public_key"));
    let secret = from_hex(field(&text, "secret_key"));

    TestKey {
        public: PublicKey::from_bytes52(&public.as_slice().try_into().expect("52 bytes"))
            .expect("the public key parses"),
        secret: SecretKey::from_ssz_bytes(&secret).expect("the secret key parses"),
    }
}

/// The quoted value of `"<name>": "..."`, from a file this test knows the shape of.
///
/// The generator writes the hex bare, with no `0x`, so the value is taken by its quotes
/// rather than by a prefix.
pub fn field<'a>(text: &'a str, name: &str) -> &'a str {
    let after = text
        .split_once(&format!("\"{name}\""))
        .unwrap_or_else(|| panic!("no {name} in the key file"))
        .1;
    let after = after.split_once(':').expect("a key/value separator").1;
    let start = after.find('"').expect("an opening quote") + 1;
    let rest = &after[start..];
    let end = rest.find('"').expect("a closing quote");
    &rest[..end]
}

pub fn from_hex(text: &str) -> Vec<u8> {
    let text = text.strip_prefix("0x").unwrap_or(text);
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
        .collect()
}

/// Writes the genesis file, the assignment, and the key directory a node starts from.
///
/// The layout is lean-quickstart's, because that is the layout the loader reads: a
/// `hash-sig-keys/` directory with a manifest declaring both roles, and file names derived
/// from the validator index and the role.
pub fn write_configuration(root: &Path, keys: &[TestKey], genesis_time: u64) -> (PathBuf, PathBuf) {
    let key_base = root.join("keys");
    let key_directory = key_base.join("hash-sig-keys");
    fs::create_dir_all(&key_directory).expect("the key directory");

    let attestation = hex(&keys[0].public.to_bytes52());
    let proposal = hex(&keys[1].public.to_bytes52());

    let genesis_path = root.join("genesis.yaml");
    fs::write(
        &genesis_path,
        format!(
            "GENESIS_TIME: {genesis_time}\nGENESIS_VALIDATORS:\n  - attestation_public_key: \"0x{attestation}\"\n    proposal_public_key: \"0x{proposal}\"\n"
        ),
    )
    .expect("the genesis file");

    fs::write(key_base.join("validators.yaml"), "verity_0:\n  - 0\n").expect("the assignment file");

    fs::write(
        key_directory.join("validator-keys-manifest.yaml"),
        format!(
            "validators:\n  - attester_key_pubkey_hex: \"0x{attestation}\"\n    proposer_key_pubkey_hex: \"0x{proposal}\"\n"
        ),
    )
    .expect("the key manifest");

    for (role, key) in [("attester", &keys[0]), ("proposer", &keys[1])] {
        fs::write(
            key_directory.join(format!("validator_0_{role}_key_pk.ssz")),
            key.public.to_bytes52(),
        )
        .expect("a public key file");
        fs::write(
            key_directory.join(format!("validator_0_{role}_key_sk.ssz")),
            key.secret.to_ssz_bytes(),
        )
        .expect("a secret key file");
    }

    (genesis_path, key_base)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
}
