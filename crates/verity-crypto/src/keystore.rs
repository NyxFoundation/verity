//! Loading validator key material from a lean-quickstart genesis directory.
//!
//! # Why this loader exists at all
//!
//! No loader for the ecosystem's key-file layout exists in any upstream repository. leanSig
//! ships none, and `leansig-test-keys` is data rather than code. Every lean client has
//! written its own, so Verity writes one too — but against the same de-facto standard, since
//! interoperating with lean-quickstart's generator is the entire point.
//!
//! # The layout
//!
//! ```text
//! <genesis>/hash-sig-keys/
//!     validator-keys-manifest.yaml
//!     validator_<i>_attester_key_pk.ssz
//!     validator_<i>_attester_key_sk.ssz
//!     validator_<i>_proposer_key_pk.ssz
//!     validator_<i>_proposer_key_sk.ssz
//! ```
//!
//! The manifest is authoritative: it declares each validator's two public keys as hex, and a
//! key file that disagrees with it is a reason to stop, not a reason to prefer the file. The
//! file *names* are derived from the index and the role rather than searched for by
//! substring, so a stray file cannot be mistaken for a key.
//!
//! # Fail-closed, every time
//!
//! Missing either role's key, a key that disagrees with the manifest, a manifest declaring
//! one key per validator, or two roles sharing a key: all refuse to start. There is no
//! degraded mode. The last two are worth naming — zeam falls back to one key covering both
//! roles when only one file is present, and that is precisely the dual-role reuse that
//! breaks a key rather than costing a duty.
//!
//! # What this cannot check
//!
//! Nothing here proves that a `_sk.ssz` file is the secret half of the `_pk.ssz` file beside
//! it. leanSig exposes no way to derive the public key from the secret one, and the only
//! other proof — signing something and verifying it — would consume an epoch of the very key
//! it was checking. The binding is taken on the generator's word, as it is in every other
//! lean client.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use verity_types::{Bytes52, ValidatorIndex};

use crate::containers::PublicKey;
use crate::error::KeyLoadError;
use crate::key::SecretKey;
use crate::scheme::Role;

/// File name of the manifest inside the key directory.
pub const MANIFEST_FILE_NAME: &str = "validator-keys-manifest.yaml";

/// One role's key pair for one validator.
#[derive(Debug)]
pub struct RoleKeys {
    /// The public key, agreed between the manifest and the key file.
    pub public: PublicKey,
    /// The secret key this node signs with.
    pub secret: SecretKey,
}

/// Everything one validator needs in order to perform both of its duties.
#[derive(Debug)]
pub struct ValidatorKeys {
    /// The validator's position in the registry.
    pub index: ValidatorIndex,
    /// Signs attestations.
    pub attestation: RoleKeys,
    /// Signs block roots.
    pub proposal: RoleKeys,
}

impl ValidatorKeys {
    /// The key pair for one role.
    pub fn role(&self, role: Role) -> &RoleKeys {
        match role {
            Role::Attestation => &self.attestation,
            Role::Proposal => &self.proposal,
        }
    }
}

/// Loads the keys for the validators this node is responsible for.
///
/// Which validators those are is the caller's decision — it comes from the node's duty
/// assignment, which this crate has no view of. Passing an index the manifest does not cover
/// is an error rather than a silent omission.
///
/// # Memory
///
/// Keys are held whole in memory, like every surveyed client. A production-scheme secret key
/// is about 33.5 MB and each validator has two, so the cost is roughly 67 MB per validator —
/// about 4.3 GB for 64. That bound is linear and known; mapping or partial loading is not
/// built until something measures a need for it.
///
/// # Errors
///
/// Every variant of [`KeyLoadError`]. All of them mean the node must not start.
pub fn load(
    key_directory: &Path,
    indices: &[ValidatorIndex],
) -> Result<Vec<ValidatorKeys>, KeyLoadError> {
    let manifest_path = key_directory.join(MANIFEST_FILE_NAME);
    let manifest = Manifest::read(&manifest_path)?;

    indices
        .iter()
        .map(|index| load_validator(key_directory, &manifest, &manifest_path, *index))
        .collect()
}

fn load_validator(
    key_directory: &Path,
    manifest: &Manifest,
    manifest_path: &Path,
    index: ValidatorIndex,
) -> Result<ValidatorKeys, KeyLoadError> {
    let entry = manifest
        .entry(index)
        .ok_or(KeyLoadError::UnknownValidator { index: index.0 })?;

    // Both declarations first, and the dual-role check before either secret key is opened. A
    // manifest that covers both roles with one key is wrong whatever is on disk, and finding
    // that out costs two hex strings rather than 67 MB of reads.
    let declared_attestation = entry.public_key(Role::Attestation, manifest_path, index)?;
    let declared_proposal = entry.public_key(Role::Proposal, manifest_path, index)?;
    if declared_attestation == declared_proposal {
        return Err(KeyLoadError::DuplicateRoleKeys { index: index.0 });
    }

    Ok(ValidatorKeys {
        index,
        attestation: load_role(
            key_directory,
            index,
            Role::Attestation,
            declared_attestation,
        )?,
        proposal: load_role(key_directory, index, Role::Proposal, declared_proposal)?,
    })
}

fn load_role(
    key_directory: &Path,
    index: ValidatorIndex,
    role: Role,
    declared: PublicKey,
) -> Result<RoleKeys, KeyLoadError> {
    let public_path = key_file(key_directory, index, role, "pk");
    let public = read_public_key(&public_path)?;
    if public != declared {
        return Err(KeyLoadError::PublicKeyMismatch {
            index: index.0,
            role,
        });
    }

    let secret_path = key_file(key_directory, index, role, "sk");
    let secret = SecretKey::from_ssz_bytes(&read(&secret_path)?)
        .map_err(|()| KeyLoadError::MalformedKeyFile { path: secret_path })?;

    Ok(RoleKeys { public, secret })
}

fn key_file(key_directory: &Path, index: ValidatorIndex, role: Role, half: &str) -> PathBuf {
    key_directory.join(format!(
        "validator_{}_{}_key_{half}.ssz",
        index.0,
        role.file_infix()
    ))
}

fn read(path: &Path) -> Result<Vec<u8>, KeyLoadError> {
    fs::read(path).map_err(|error| KeyLoadError::Unreadable {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}

fn read_public_key(path: &Path) -> Result<PublicKey, KeyLoadError> {
    let bytes = read(path)?;
    let bytes: Bytes52 =
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| KeyLoadError::MalformedKeyFile {
                path: path.to_path_buf(),
            })?;

    PublicKey::from_bytes52(&bytes).map_err(|_| KeyLoadError::MalformedKeyFile {
        path: path.to_path_buf(),
    })
}

/// The manifest, reduced to the two fields Verity reads.
///
/// Every other field the generator writes is ignored rather than rejected: the manifest is
/// shared tooling output and gains fields over time, and refusing to start over one Verity
/// does not read would make the node fragile against changes that cannot affect it.
#[derive(Debug, Deserialize)]
struct Manifest {
    validators: Vec<ManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    /// Hex of the attestation public key. Absent in the pre-devnet4 single-key layout.
    #[serde(default)]
    attester_key_pubkey_hex: Option<String>,
    /// Hex of the proposal public key. Absent in the pre-devnet4 single-key layout.
    #[serde(default)]
    proposer_key_pubkey_hex: Option<String>,
}

impl Manifest {
    fn read(path: &Path) -> Result<Self, KeyLoadError> {
        let text = fs::read_to_string(path).map_err(|error| KeyLoadError::Unreadable {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;

        let manifest: Self =
            serde_norway::from_str(&text).map_err(|error| KeyLoadError::MalformedManifest {
                path: path.to_path_buf(),
                reason: error.to_string(),
            })?;

        // One entry decides it for the whole file: the generator writes a uniform layout, and
        // a mixed one would mean something other than the generator produced it.
        if manifest
            .validators
            .first()
            .is_some_and(|entry| entry.attester_key_pubkey_hex.is_none())
        {
            return Err(KeyLoadError::SingleKeyManifest {
                path: path.to_path_buf(),
            });
        }

        Ok(manifest)
    }

    fn entry(&self, index: ValidatorIndex) -> Option<&ManifestEntry> {
        usize::try_from(index.0)
            .ok()
            .and_then(|index| self.validators.get(index))
    }
}

impl ManifestEntry {
    fn public_key(
        &self,
        role: Role,
        manifest_path: &Path,
        index: ValidatorIndex,
    ) -> Result<PublicKey, KeyLoadError> {
        let hex = match role {
            Role::Attestation => self.attester_key_pubkey_hex.as_deref(),
            Role::Proposal => self.proposer_key_pubkey_hex.as_deref(),
        }
        .ok_or_else(|| KeyLoadError::SingleKeyManifest {
            path: manifest_path.to_path_buf(),
        })?;

        let malformed = || KeyLoadError::MalformedManifestKey {
            index: index.0,
            role,
        };

        let bytes = decode_hex(hex).ok_or_else(malformed)?;
        let bytes: Bytes52 = bytes.as_slice().try_into().map_err(|_| malformed())?;
        PublicKey::from_bytes52(&bytes).map_err(|_| malformed())
    }
}

/// Decodes hex with an optional `0x` prefix.
///
/// Written here rather than pulled in: the manifest is the only hex this crate reads, and the
/// rule it needs — even length, `0x` optional, no whitespace — is narrower than any general
/// decoder's.
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    let digits = text.strip_prefix("0x").unwrap_or(text);
    if !digits.len().is_multiple_of(2) {
        return None;
    }

    digits
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()
        })
        .collect()
}

/// A key directory reduced to what a caller can learn without loading 33.5 MB per key.
///
/// Reading the manifest alone answers "which validators does this directory cover, and what
/// are their public keys" — which is what genesis construction and operator tooling want,
/// and neither of them wants the secret material.
///
/// # Errors
///
/// The manifest-related variants of [`KeyLoadError`]; no key file is opened.
pub fn read_public_keys(
    key_directory: &Path,
) -> Result<BTreeMap<ValidatorIndex, [PublicKey; 2]>, KeyLoadError> {
    let manifest_path = key_directory.join(MANIFEST_FILE_NAME);
    let manifest = Manifest::read(&manifest_path)?;

    manifest
        .validators
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            let index = ValidatorIndex(position as u64);
            let attestation = entry.public_key(Role::Attestation, &manifest_path, index)?;
            let proposal = entry.public_key(Role::Proposal, &manifest_path, index)?;
            if attestation == proposal {
                return Err(KeyLoadError::DuplicateRoleKeys { index: index.0 });
            }
            Ok((index, [attestation, proposal]))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use libssz::SszEncode;
    use verity_types::ValidatorIndex;

    use super::{MANIFEST_FILE_NAME, decode_hex, load, read_public_keys};
    use crate::containers::{Fp, HashDigest, Parameter, PublicKey};
    use crate::error::KeyLoadError;
    use crate::scheme::Role;

    /// A temporary directory that removes itself, so a failing test leaves nothing behind.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("verity-crypto-{name}"));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, name: &str, contents: impl AsRef<[u8]>) {
            fs::write(self.0.join(name), contents).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn public_key(seed: u32) -> PublicKey {
        PublicKey {
            root: HashDigest::try_from((0..8).map(|i| Fp(seed + i)).collect::<Vec<_>>()).unwrap(),
            parameter: Parameter::try_from((0..5).map(|i| Fp(seed + 100 + i)).collect::<Vec<_>>())
                .unwrap(),
        }
    }

    fn hex_of(key: &PublicKey) -> String {
        let mut out = String::from("0x");
        for byte in key.to_ssz() {
            out.push_str(&format!("{byte:02x}"));
        }
        out
    }

    fn dual_key_manifest(entries: &[(PublicKey, PublicKey)]) -> String {
        let mut yaml = String::from("validators:\n");
        for (attester, proposer) in entries {
            yaml.push_str(&format!(
                "  - attester_key_pubkey_hex: \"{}\"\n    proposer_key_pubkey_hex: \"{}\"\n",
                hex_of(attester),
                hex_of(proposer)
            ));
        }
        yaml
    }

    fn pk_file_name(index: u64, role: Role) -> String {
        format!("validator_{index}_{}_key_pk.ssz", role.file_infix())
    }

    fn sk_file_name(index: u64, role: Role) -> String {
        format!("validator_{index}_{}_key_sk.ssz", role.file_infix())
    }

    #[test]
    fn should_decode_hex_with_or_without_a_prefix_and_reject_anything_else() {
        assert_eq!(decode_hex("0x00ff"), Some(vec![0x00, 0xff]));
        assert_eq!(decode_hex("00FF"), Some(vec![0x00, 0xff]));
        assert_eq!(decode_hex("0xf"), None, "odd length");
        assert_eq!(decode_hex("0xzz"), None, "not hex");
        assert_eq!(decode_hex("0x 0"), None, "whitespace");
    }

    #[test]
    fn should_refuse_when_the_manifest_declares_one_key_per_validator() {
        let dir = TempDir::new("legacy-manifest");
        dir.write(
            MANIFEST_FILE_NAME,
            "validators:\n  - pubkey_hex: \"0x00\"\n",
        );

        assert!(matches!(
            load(dir.path(), &[ValidatorIndex(0)]),
            Err(KeyLoadError::SingleKeyManifest { .. })
        ));
    }

    #[test]
    fn should_refuse_when_the_manifest_has_no_entry_for_a_requested_validator() {
        let dir = TempDir::new("short-manifest");
        dir.write(
            MANIFEST_FILE_NAME,
            dual_key_manifest(&[(public_key(1), public_key(2))]),
        );

        assert_eq!(
            load(dir.path(), &[ValidatorIndex(1)]).unwrap_err(),
            KeyLoadError::UnknownValidator { index: 1 }
        );
    }

    #[test]
    fn should_refuse_when_one_validator_uses_one_key_for_both_roles() {
        let dir = TempDir::new("duplicate-roles");
        let shared = public_key(9);
        dir.write(
            MANIFEST_FILE_NAME,
            dual_key_manifest(&[(shared.clone(), shared)]),
        );

        assert_eq!(
            load(dir.path(), &[ValidatorIndex(0)]).unwrap_err(),
            KeyLoadError::DuplicateRoleKeys { index: 0 }
        );
    }

    #[test]
    fn should_refuse_when_a_manifest_key_is_not_a_valid_public_key() {
        let dir = TempDir::new("bad-manifest-key");
        dir.write(
            MANIFEST_FILE_NAME,
            format!(
                "validators:\n  - attester_key_pubkey_hex: \"0xdead\"\n    proposer_key_pubkey_hex: \"{}\"\n",
                hex_of(&public_key(2))
            ),
        );

        assert_eq!(
            load(dir.path(), &[ValidatorIndex(0)]).unwrap_err(),
            KeyLoadError::MalformedManifestKey {
                index: 0,
                role: Role::Attestation,
            }
        );
    }

    #[test]
    fn should_refuse_when_a_key_file_is_missing() {
        let dir = TempDir::new("missing-key-file");
        dir.write(
            MANIFEST_FILE_NAME,
            dual_key_manifest(&[(public_key(1), public_key(2))]),
        );

        assert!(matches!(
            load(dir.path(), &[ValidatorIndex(0)]),
            Err(KeyLoadError::Unreadable { .. })
        ));
    }

    #[test]
    fn should_refuse_when_a_key_file_disagrees_with_the_manifest() {
        let dir = TempDir::new("key-file-mismatch");
        dir.write(
            MANIFEST_FILE_NAME,
            dual_key_manifest(&[(public_key(1), public_key(2))]),
        );
        // The manifest is authoritative, so a different-but-valid key on disk is a stop.
        dir.write(&pk_file_name(0, Role::Attestation), public_key(3).to_ssz());

        assert_eq!(
            load(dir.path(), &[ValidatorIndex(0)]).unwrap_err(),
            KeyLoadError::PublicKeyMismatch {
                index: 0,
                role: Role::Attestation,
            }
        );
    }

    #[test]
    fn should_refuse_when_a_secret_key_file_does_not_decode() {
        let dir = TempDir::new("bad-secret-key");
        let attester = public_key(1);
        dir.write(
            MANIFEST_FILE_NAME,
            dual_key_manifest(&[(attester.clone(), public_key(2))]),
        );
        dir.write(&pk_file_name(0, Role::Attestation), attester.to_ssz());
        dir.write(&sk_file_name(0, Role::Attestation), b"not a secret key");

        assert!(matches!(
            load(dir.path(), &[ValidatorIndex(0)]),
            Err(KeyLoadError::MalformedKeyFile { .. })
        ));
    }

    #[test]
    fn should_report_the_declared_keys_without_opening_a_key_file() {
        let dir = TempDir::new("public-keys-only");
        let entries = [
            (public_key(1), public_key(2)),
            (public_key(3), public_key(4)),
        ];
        dir.write(MANIFEST_FILE_NAME, dual_key_manifest(&entries));

        let keys = read_public_keys(dir.path()).unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[&ValidatorIndex(1)], [public_key(3), public_key(4)]);
    }
}
