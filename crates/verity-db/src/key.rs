//! How a consensus identifier becomes a database key.
//!
//! Two rules hold everywhere. Roots are stored raw, as their 32 SSZ bytes. Integers are
//! stored fixed-width **big-endian**, so lexicographic key order — the only order a
//! byte-level backend has — is numeric order. That equality is what makes a slot range a
//! contiguous scan and a proof expiry a single range tombstone.
//!
//! Transcribed from `docs/design/storage.md`, "Column families".

use verity_types::primitives::{Bytes32, Slot, ValidatorIndex};

use crate::column::ColumnFamily;
use crate::error::StorageError;

/// Width of a big-endian `u64` key component.
pub const SLOT_WIDTH: usize = 8;

/// Width of a raw 32-byte root key component.
pub const ROOT_WIDTH: usize = 32;

/// Width of a `slot_be ‖ root` composite key.
pub const SLOT_ROOT_WIDTH: usize = SLOT_WIDTH + ROOT_WIDTH;

/// A root, as its own key.
#[must_use]
pub const fn root(root: Bytes32) -> [u8; ROOT_WIDTH] {
    root
}

/// A slot, as a big-endian key.
#[must_use]
pub fn slot(slot: Slot) -> [u8; SLOT_WIDTH] {
    slot.0.to_be_bytes()
}

/// A validator index, as a big-endian key.
#[must_use]
pub fn validator(index: ValidatorIndex) -> [u8; SLOT_WIDTH] {
    index.0.to_be_bytes()
}

/// The `slot_be ‖ root` key that orders a table by slot first.
#[must_use]
pub fn slot_and_root(at: Slot, block_root: Bytes32) -> [u8; SLOT_ROOT_WIDTH] {
    let mut key = [0u8; SLOT_ROOT_WIDTH];
    key[..SLOT_WIDTH].copy_from_slice(&slot(at));
    key[SLOT_WIDTH..].copy_from_slice(&block_root);
    key
}

/// The half-open key bounds covering slots `[start, end)` in a slot-keyed table.
#[must_use]
pub fn slot_bounds(start: Slot, end: Slot) -> ([u8; SLOT_WIDTH], [u8; SLOT_WIDTH]) {
    (slot(start), slot(end))
}

/// The half-open key bounds covering slots `[start, end)` in a `slot_be ‖ root` table.
///
/// The zero root is the smallest 32-byte suffix, so a bound of `slot ‖ 0` sits at or before
/// every key at that slot and strictly after every key below it. The range therefore covers
/// exactly the slots asked for, whatever roots happen to be present.
#[must_use]
pub fn slot_and_root_bounds(
    start: Slot,
    end: Slot,
) -> ([u8; SLOT_ROOT_WIDTH], [u8; SLOT_ROOT_WIDTH]) {
    (
        slot_and_root(start, [0u8; ROOT_WIDTH]),
        slot_and_root(end, [0u8; ROOT_WIDTH]),
    )
}

/// Reads back a big-endian slot key.
///
/// # Errors
///
/// [`StorageError::KeyWidth`] when the stored key is not eight bytes wide, which means the
/// table holds a key nothing in this module could have written.
pub fn decode_slot(table: ColumnFamily, key: &[u8]) -> Result<Slot, StorageError> {
    let bytes: [u8; SLOT_WIDTH] = key.try_into().map_err(|_| StorageError::KeyWidth {
        table,
        expected: SLOT_WIDTH,
        found: key.len(),
    })?;
    Ok(Slot(u64::from_be_bytes(bytes)))
}

/// Reads back a big-endian validator-index key.
///
/// # Errors
///
/// [`StorageError::KeyWidth`] when the stored key is not eight bytes wide.
pub fn decode_validator(table: ColumnFamily, key: &[u8]) -> Result<ValidatorIndex, StorageError> {
    let bytes: [u8; SLOT_WIDTH] = key.try_into().map_err(|_| StorageError::KeyWidth {
        table,
        expected: SLOT_WIDTH,
        found: key.len(),
    })?;
    Ok(ValidatorIndex(u64::from_be_bytes(bytes)))
}

/// Reads back a raw root key.
///
/// # Errors
///
/// [`StorageError::KeyWidth`] when the stored key is not 32 bytes wide.
pub fn decode_root(table: ColumnFamily, key: &[u8]) -> Result<Bytes32, StorageError> {
    key.try_into().map_err(|_| StorageError::KeyWidth {
        table,
        expected: ROOT_WIDTH,
        found: key.len(),
    })
}

/// Reads back a `slot_be ‖ root` composite key.
///
/// # Errors
///
/// [`StorageError::KeyWidth`] when the stored key is not 40 bytes wide.
pub fn decode_slot_and_root(
    table: ColumnFamily,
    key: &[u8],
) -> Result<(Slot, Bytes32), StorageError> {
    if key.len() != SLOT_ROOT_WIDTH {
        return Err(StorageError::KeyWidth {
            table,
            expected: SLOT_ROOT_WIDTH,
            found: key.len(),
        });
    }
    let at = decode_slot(table, &key[..SLOT_WIDTH])?;
    let block_root = decode_root(table, &key[SLOT_WIDTH..])?;
    Ok((at, block_root))
}

#[cfg(test)]
mod tests {
    use verity_types::primitives::Slot;

    use super::{ColumnFamily, decode_slot_and_root, slot, slot_and_root, slot_and_root_bounds};

    #[test]
    fn should_order_slot_keys_numerically_when_compared_as_bytes() {
        assert!(slot(Slot(255)) < slot(Slot(256)), "big-endian, not little");
        assert!(slot(Slot(0)) < slot(Slot(u64::MAX)));
    }

    #[test]
    fn should_group_every_root_at_a_slot_inside_that_slot_bounds() {
        let (start, end) = slot_and_root_bounds(Slot(7), Slot(8));
        for byte in [0x00u8, 0x7f, 0xff] {
            let key = slot_and_root(Slot(7), [byte; 32]);
            assert!(key >= start && key < end, "root {byte:#x} left slot 7");
        }
        assert!(slot_and_root(Slot(8), [0u8; 32]) >= end);
        assert!(slot_and_root(Slot(6), [0xffu8; 32]) < start);
    }

    #[test]
    fn should_round_trip_a_composite_key() {
        let key = slot_and_root(Slot(4_096), [3u8; 32]);
        assert_eq!(
            decode_slot_and_root(ColumnFamily::BlockProofs, &key).unwrap(),
            (Slot(4_096), [3u8; 32])
        );
    }

    #[test]
    fn should_refuse_a_composite_key_of_the_wrong_width() {
        assert!(decode_slot_and_root(ColumnFamily::BlockProofs, &[0u8; 39]).is_err());
    }
}

#[cfg(kani)]
mod harnesses {
    use verity_types::primitives::{Bytes32, Slot, ValidatorIndex};

    use super::{
        ColumnFamily, ROOT_WIDTH, SLOT_ROOT_WIDTH, SLOT_WIDTH, StorageError, decode_root,
        decode_slot, decode_slot_and_root, decode_validator, root, slot, slot_and_root,
        slot_and_root_bounds, slot_bounds, validator,
    };

    /// Byte order on slot keys is numeric order: the property every range scan rests on.
    #[kani::proof]
    fn slot_keys_order_like_slots() {
        let a: u64 = kani::any();
        let b: u64 = kani::any();
        assert!((slot(Slot(a)) < slot(Slot(b))) == (a < b));
        assert!((slot(Slot(a)) == slot(Slot(b))) == (a == b));
        assert!((validator(ValidatorIndex(a)) < validator(ValidatorIndex(b))) == (a < b));
    }

    /// Composite keys order by slot first, and every root at a slot sits inside that slot's
    /// bounds and outside every other slot's.
    #[kani::proof]
    fn composite_keys_order_by_slot_first() {
        let a: u64 = kani::any();
        let b: u64 = kani::any();
        let root_a: Bytes32 = kani::any();
        let root_b: Bytes32 = kani::any();
        let key_a = slot_and_root(Slot(a), root_a);
        let key_b = slot_and_root(Slot(b), root_b);
        if a < b {
            assert!(key_a < key_b);
        }
        if a == b {
            assert!((key_a < key_b) == (root_a < root_b));
        }

        let start: u64 = kani::any();
        let end: u64 = kani::any();
        let (low, high) = slot_and_root_bounds(Slot(start), Slot(end));
        let inside = start <= a && a < end;
        assert!((low <= key_a && key_a < high) == inside);
        let (low, high) = slot_bounds(Slot(start), Slot(end));
        assert!((low <= slot(Slot(a)) && slot(Slot(a)) < high) == inside);
    }

    /// A slot key and a validator key read back as what was written.
    #[kani::proof]
    fn integer_keys_read_back() {
        let table = ColumnFamily::BlockProofs;
        let at: u64 = kani::any();
        let index: u64 = kani::any();
        assert!(decode_slot(table, &slot(Slot(at))).unwrap().0 == at);
        assert!(
            decode_validator(table, &validator(ValidatorIndex(index)))
                .unwrap()
                .0
                == index
        );
    }

    /// A root key reads back as what was written.
    #[kani::proof]
    fn root_keys_read_back() {
        let table = ColumnFamily::BlockProofs;
        let block_root: Bytes32 = kani::any();
        assert!(decode_root(table, &root(block_root)).unwrap() == block_root);
    }

    /// A composite key reads back as the slot and root it was written from.
    #[kani::proof]
    fn composite_keys_read_back() {
        let table = ColumnFamily::BlockProofs;
        let at: u64 = kani::any();
        let block_root: Bytes32 = kani::any();
        let (read_at, read_root) =
            decode_slot_and_root(table, &slot_and_root(Slot(at), block_root)).unwrap();
        assert!(read_at.0 == at);
        assert!(read_root == block_root);
    }

    /// A key of any other width is refused by name, never indexed.
    #[kani::proof]
    #[kani::unwind(43)]
    fn foreign_widths_are_refused() {
        let table = ColumnFamily::BlockProofs;
        let len: usize = kani::any();
        kani::assume(len <= SLOT_ROOT_WIDTH + 1);
        let key: Vec<u8> = (0..len).map(|_| kani::any()).collect();
        let width_error = |expected: usize, outcome: Result<(), StorageError>| match outcome {
            Ok(()) => assert!(key.len() == expected),
            Err(StorageError::KeyWidth {
                found,
                expected: reported,
                ..
            }) => {
                assert!(key.len() != expected);
                assert!(found == key.len());
                assert!(reported == expected);
            }
            Err(_) => unreachable!("only the width is checked"),
        };
        width_error(SLOT_WIDTH, decode_slot(table, &key).map(|_| ()));
        width_error(SLOT_WIDTH, decode_validator(table, &key).map(|_| ()));
        width_error(ROOT_WIDTH, decode_root(table, &key).map(|_| ()));
        width_error(
            SLOT_ROOT_WIDTH,
            decode_slot_and_root(table, &key).map(|_| ()),
        );
    }
}
