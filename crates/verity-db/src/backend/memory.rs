//! The in-memory sibling of the RocksDB backend.
//!
//! Used by tests and by ephemeral nodes. It is a `BTreeMap` per table precisely because
//! `BTreeMap<Vec<u8>, _>` iterates in the same lexicographic byte order RocksDB does: a range
//! scan written against this backend returns the rows, in the order, production returns.
//!
//! Durability is meaningless here — nothing survives the process — so [`Durability`] is
//! accepted and ignored rather than being absent from the contract.

use std::collections::BTreeMap;

use crate::column::ColumnFamily;
use crate::error::StorageError;

use super::{Durability, Op, Rows, StorageBackend, WriteBatch};

/// A volatile backend holding every table in a sorted map.
#[derive(Debug, Clone, Default)]
pub struct MemoryBackend {
    tables: BTreeMap<ColumnFamily, BTreeMap<Vec<u8>, Vec<u8>>>,
}

impl MemoryBackend {
    /// An empty database with every column family present.
    #[must_use]
    pub fn new() -> Self {
        Self {
            tables: ColumnFamily::ALL
                .into_iter()
                .map(|table| (table, BTreeMap::new()))
                .collect(),
        }
    }

    /// How many rows a table holds. Test affordance; the repository does not count rows.
    #[must_use]
    pub fn row_count(&self, table: ColumnFamily) -> usize {
        self.tables.get(&table).map_or(0, BTreeMap::len)
    }

    fn table_mut(&mut self, table: ColumnFamily) -> &mut BTreeMap<Vec<u8>, Vec<u8>> {
        self.tables.entry(table).or_default()
    }

    fn apply(&mut self, op: Op) {
        match op {
            Op::Put { table, key, value } => {
                self.table_mut(table).insert(key, value);
            }
            Op::Delete { table, key } => {
                self.table_mut(table).remove(&key);
            }
            Op::DeleteRange { table, start, end } => {
                let doomed: Vec<Vec<u8>> = self
                    .table_mut(table)
                    .range(start..end)
                    .map(|(key, _)| key.clone())
                    .collect();
                let rows = self.table_mut(table);
                for key in doomed {
                    rows.remove(&key);
                }
            }
        }
    }
}

impl StorageBackend for MemoryBackend {
    fn get(&self, table: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self
            .tables
            .get(&table)
            .and_then(|rows| rows.get(key))
            .cloned())
    }

    fn range(&self, table: ColumnFamily, start: &[u8], end: &[u8]) -> Result<Rows, StorageError> {
        // A reversed interval is empty, not an error, matching a RocksDB iterator positioned past
        // its upper bound.
        if start >= end {
            return Ok(Vec::new());
        }
        Ok(self.tables.get(&table).map_or_else(Vec::new, |rows| {
            rows.range(start.to_vec()..end.to_vec())
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        }))
    }

    fn write(&mut self, batch: WriteBatch, _durability: Durability) -> Result<(), StorageError> {
        // Atomicity is free: nothing observes the map between two ops of one `write` call,
        // because the writer holds the only mutable handle.
        for op in batch.ops().iter().cloned() {
            self.apply(op);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ColumnFamily, Durability, MemoryBackend, StorageBackend, WriteBatch};

    fn seeded() -> MemoryBackend {
        let mut backend = MemoryBackend::new();
        let mut batch = WriteBatch::new();
        for slot in 0u64..8 {
            batch.queue_put(
                ColumnFamily::CanonicalBlocks,
                slot.to_be_bytes().to_vec(),
                vec![u8::try_from(slot).unwrap()],
            );
        }
        backend.write(batch, Durability::Buffered).unwrap();
        backend
    }

    #[test]
    fn should_return_a_range_in_ascending_key_order() {
        let backend = seeded();
        let rows = backend
            .range(
                ColumnFamily::CanonicalBlocks,
                &2u64.to_be_bytes(),
                &5u64.to_be_bytes(),
            )
            .unwrap();
        let values: Vec<u8> = rows.iter().map(|(_, value)| value[0]).collect();
        assert_eq!(values, vec![2, 3, 4], "half-open, and sorted");
    }

    #[test]
    fn should_treat_a_reversed_interval_as_empty() {
        let backend = seeded();
        let rows = backend
            .range(
                ColumnFamily::CanonicalBlocks,
                &5u64.to_be_bytes(),
                &2u64.to_be_bytes(),
            )
            .unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn should_apply_a_range_delete_to_exactly_the_half_open_interval() {
        let mut backend = seeded();
        let mut batch = WriteBatch::new();
        batch.queue_delete_range(
            ColumnFamily::CanonicalBlocks,
            0u64.to_be_bytes().to_vec(),
            3u64.to_be_bytes().to_vec(),
        );
        backend.write(batch, Durability::Buffered).unwrap();

        assert_eq!(backend.row_count(ColumnFamily::CanonicalBlocks), 5);
        assert!(
            backend
                .get(ColumnFamily::CanonicalBlocks, &3u64.to_be_bytes())
                .unwrap()
                .is_some(),
            "the exclusive upper bound survives"
        );
    }

    #[test]
    fn should_let_a_later_op_in_a_batch_win_over_an_earlier_one() {
        let mut backend = MemoryBackend::new();
        let mut batch = WriteBatch::new();
        batch.queue_put(ColumnFamily::Metadata, b"head".to_vec(), vec![1]);
        batch.queue_put(ColumnFamily::Metadata, b"head".to_vec(), vec![2]);
        backend.write(batch, Durability::Synced).unwrap();
        assert_eq!(
            backend.get(ColumnFamily::Metadata, b"head").unwrap(),
            Some(vec![2])
        );
    }

    #[test]
    fn should_report_no_value_for_an_absent_key() {
        assert_eq!(
            MemoryBackend::new()
                .get(ColumnFamily::Metadata, b"head")
                .unwrap(),
            None
        );
    }
}
