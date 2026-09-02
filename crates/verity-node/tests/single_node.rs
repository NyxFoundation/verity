//! One node, real keys, real proofs: does the chain actually move?
//!
//! This is the end-to-end check the unit tests cannot make. Every stage below is the real
//! one — the wall clock, the duty loop, XMSS signing, leanVM proving, the verification the
//! chain task's own imports skip, RocksDB — and the only thing asserted is the outcome an
//! operator would look for: the head left genesis.
//!
//! # Gated, and slow on purpose
//!
//! It needs production-scheme keys, which are 33.5 MB each and cannot be shrunk without
//! changing the parameters Verity runs. `VERITY_TEST_KEYS` supplies them
//! (<https://github.com/leanEthereum/leansig-test-keys>, `prod_scheme.tar.gz`, sha256-pinned
//! in `crates/verity-crypto/test-keys.sha256`); with the variable unset this test returns
//! immediately, which is what keeps the fast `cargo test` gate fast. Building one block proof
//! is seconds of zk proving, so the budget below is generous by design.
//!
//! The keys are attestation-role only. Roles differ in which duty signs with them, not in the
//! cryptography, so validator 0 is given key 0 for attesting and key 1 for proposing — two
//! distinct keys, which is what the loader insists on.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use verity_crypto::SecretKey;
use verity_crypto::containers::PublicKey;
use verity_node::{Node, NodeConfig, config::GenesisFile, identity::Keypair};
use verity_types::config::SECONDS_PER_SLOT;

/// How long the node is given to produce, prove, and import its first block.
///
/// Bounded by proving, not by consensus: a single-signature aggregate and its merge are the
/// two slowest things in the run, and both happen once here.
const BUDGET: Duration = Duration::from_secs(600);

/// A key pair as `leansig-test-keys` ships it.
struct TestKey {
    public: PublicKey,
    secret: SecretKey,
}

/// Loads the first two key files in `VERITY_TEST_KEYS`, or `None` when the gate is off.
fn test_keys() -> Option<Vec<TestKey>> {
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

fn read_key(path: &PathBuf) -> TestKey {
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
fn field<'a>(text: &'a str, name: &str) -> &'a str {
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

fn from_hex(text: &str) -> Vec<u8> {
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
fn write_configuration(root: &Path, keys: &[TestKey], genesis_time: u64) -> (PathBuf, PathBuf) {
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn should_advance_the_head_past_genesis_when_a_node_runs_its_own_validator() {
    let Some(keys) = test_keys() else {
        eprintln!("skipping: set VERITY_TEST_KEYS to run the single-node end-to-end test");
        return;
    };

    // The clock has to land inside every key's prepared window, and past slot 0, since slot 0
    // is the anchor rather than a proposal. Taking the slot from the keys rather than assuming
    // one is what keeps this honest against key material that has already been advanced.
    let first_signable = keys
        .iter()
        .map(|key| key.secret.prepared_interval().start)
        .max()
        .expect("two keys");
    let target_slot = first_signable + 1;
    assert!(
        target_slot < 16,
        "the supplied keys are prepared from slot {first_signable}; \
         starting a chain there would ask the node to tick through {target_slot} slots"
    );

    let root = tempfile::tempdir().expect("a temporary directory");
    let genesis_time = now_seconds() - target_slot * SECONDS_PER_SLOT;
    let (genesis_path, key_base) = write_configuration(root.path(), &keys, genesis_time);

    let node = Node::start(NodeConfig {
        genesis: GenesisFile::read(&genesis_path).expect("the genesis file"),
        data_directory: root.path().join("db"),
        listen: "/ip4/127.0.0.1/udp/0/quic-v1"
            .parse()
            .expect("a listen address"),
        bootnodes: Vec::new(),
        network_name: "00000000".to_string(),
        keypair: Keypair::generate_secp256k1(),
        validator_indices: vec![verity_types::ValidatorIndex(0)],
        key_directory: Some(key_base.join("hash-sig-keys")),
        is_aggregator: true,
    })
    .await
    .expect("the node starts");

    let mut view = node.view();
    let advanced = tokio::time::timeout(BUDGET, async {
        loop {
            if view.borrow_and_update().head_checkpoint().slot.0 > 0 {
                return;
            }
            view.changed().await.expect("the chain task is running");
        }
    })
    .await;

    let head = view.borrow().head_checkpoint();
    node.shutdown().await;

    assert!(
        advanced.is_ok(),
        "the head stayed at genesis for {BUDGET:?}; it is at slot {}",
        head.slot.0
    );
    assert!(head.slot.0 > 0, "the head is still the genesis anchor");
}
