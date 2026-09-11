//! Experimental FFI for the SSZ implementation from `ethereum/ssz-specs` PR #132.
//!
//! The `lean-ssz` feature builds the vendored Lean source when the pinned Lean toolchain is
//! installed. Without that toolchain the crate remains buildable, but reports the backend as
//! unavailable. All raw pointers and status-code handling are confined here.

use std::error::Error;
use std::fmt;

/// Verity consensus shapes exported by the Lean adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SszType {
    State = 1,
    Block = 2,
    BlockBody = 3,
    BlockHeader = 4,
    AttestationData = 5,
}

/// Failure to invoke the experimental Lean backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashTreeRootError {
    /// Lean 4.33.1 was not present when this crate was built.
    BackendUnavailable,
    /// The Lean decoder rejected bytes produced for the selected shape.
    InvalidEncoding,
    /// The Lean runtime could not be initialized.
    RuntimeInitialization,
}

impl fmt::Display for HashTreeRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::BackendUnavailable => "the experimental Lean SSZ backend is unavailable",
            Self::InvalidEncoding => "Lean rejected the SSZ encoding for the selected shape",
            Self::RuntimeInitialization => "the Lean runtime could not be initialized",
        };
        formatter.write_str(message)
    }
}

impl Error for HashTreeRootError {}

/// Whether this binary was linked with the Lean implementation.
#[must_use]
pub const fn is_available() -> bool {
    cfg!(verity_has_lean_ssz)
}

/// Computes a root through the vendored Lean decoder, merkleizer, and pure Lean SHA-256.
#[cfg(verity_has_lean_ssz)]
pub fn hash_tree_root(shape: SszType, encoded: &[u8]) -> Result<[u8; 32], HashTreeRootError> {
    let mut root = [0u8; 32];
    let status = unsafe {
        ffi::verity_ssz_hash_tree_root(
            shape as u8,
            encoded.as_ptr(),
            encoded.len(),
            root.as_mut_ptr(),
        )
    };
    match status {
        0 => Ok(root),
        1 => Err(HashTreeRootError::RuntimeInitialization),
        _ => Err(HashTreeRootError::InvalidEncoding),
    }
}

/// Reports an unavailable backend when the pinned Lean toolchain was absent at build time.
#[cfg(not(verity_has_lean_ssz))]
pub fn hash_tree_root(_shape: SszType, _encoded: &[u8]) -> Result<[u8; 32], HashTreeRootError> {
    Err(HashTreeRootError::BackendUnavailable)
}

#[cfg(verity_has_lean_ssz)]
mod ffi {
    unsafe extern "C" {
        pub fn verity_ssz_hash_tree_root(tag: u8, data: *const u8, len: usize, out: *mut u8)
        -> i32;
    }
}

#[cfg(test)]
mod tests {
    use super::{HashTreeRootError, SszType, hash_tree_root, is_available};

    #[test]
    fn should_report_availability_consistently_when_hashing_is_requested() {
        let result = hash_tree_root(SszType::BlockHeader, &[0; 112]);
        if is_available() {
            assert!(result.is_ok());
        } else {
            assert_eq!(result, Err(HashTreeRootError::BackendUnavailable));
        }
    }

    #[test]
    fn should_hash_from_multiple_threads_when_lean_backend_is_available() {
        if !is_available() {
            return;
        }
        let expected = hash_tree_root(SszType::BlockHeader, &[0; 112]).unwrap();
        let workers = (0..8)
            .map(|_| {
                std::thread::spawn(|| hash_tree_root(SszType::BlockHeader, &[0; 112]).unwrap())
            })
            .collect::<Vec<_>>();
        for worker in workers {
            assert_eq!(worker.join().unwrap(), expected);
        }
    }
}
