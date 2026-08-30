//! The RocksDB backend.
//!
//! The engine choice is settled in `docs/src/reference/architecture.md`: aggregate block
//! proofs are 155–236 KB each and expire in slot order, so the dominant workload is large
//! appends followed by bulk range deletion, which is what an LSM tree with range tombstones
//! is for. Everything this module adds on top of the trait is physical tuning — column
//! families, compression, sync flags — and none of it is visible to the repository.
//!
//! Write-ahead logging is left at its default, which is on. [`Durability`] chooses only
//! whether that log is fsynced before the call returns.

use std::path::{Path, PathBuf};

use rocksdb::{
    ColumnFamilyDescriptor, DB, DBCompressionType, IteratorMode, Options, ReadOptions,
    WriteBatch as RocksBatch, WriteOptions,
};

use crate::column::ColumnFamily;
use crate::error::StorageError;

use super::{Durability, Op, Rows, StorageBackend, WriteBatch};

/// A RocksDB database with one column family per logical table.
#[derive(Debug)]
pub struct RocksBackend {
    db: DB,
    path: PathBuf,
}

impl RocksBackend {
    /// Opens, creating the database and any missing column family.
    ///
    /// Creating missing column families is safe here and is not a way of tolerating a
    /// damaged database: identity checking happens a layer up, in
    /// [`crate::Repository::open`], which refuses a populated database whose metadata does
    /// not match. An empty new table in an otherwise populated directory would fail that
    /// check on its missing metadata rather than being silently adopted.
    ///
    /// # Errors
    ///
    /// [`StorageError::Backend`] when RocksDB cannot open the directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();

        let mut db_options = Options::default();
        db_options.create_if_missing(true);
        db_options.create_missing_column_families(true);

        let descriptors = ColumnFamily::ALL
            .map(|table| ColumnFamilyDescriptor::new(table.name(), table_options(table)));

        let db = DB::open_cf_descriptors(&db_options, &path, descriptors).map_err(backend)?;
        Ok(Self { db, path })
    }

    /// The directory this database lives in.
    ///
    /// A failure preserves the directory for diagnosis rather than repairing it, so the
    /// operator has to be told where to look.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn handle(&self, table: ColumnFamily) -> Result<&rocksdb::ColumnFamily, StorageError> {
        self.db.cf_handle(table.name()).ok_or_else(|| {
            StorageError::Backend(format!(
                "column family {table} is absent from {}",
                self.path.display()
            ))
        })
    }
}

/// Per-table physical options.
///
/// Compression is the only setting fixed here, because it follows from what the table holds
/// rather than from the machine. Cache size, memtable size, and compaction parallelism are
/// operational tunables and are deliberately left at RocksDB's defaults until production
/// fixtures say otherwise; none of them is a consensus constant.
fn table_options(table: ColumnFamily) -> Options {
    let mut options = Options::default();
    options.set_compression_type(if table.compressed() {
        DBCompressionType::Lz4
    } else {
        DBCompressionType::None
    });
    options
}

fn backend(error: rocksdb::Error) -> StorageError {
    StorageError::Backend(error.to_string())
}

impl StorageBackend for RocksBackend {
    fn get(&self, table: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        self.db.get_cf(self.handle(table)?, key).map_err(backend)
    }

    fn range(&self, table: ColumnFamily, start: &[u8], end: &[u8]) -> Result<Rows, StorageError> {
        if start >= end {
            return Ok(Vec::new());
        }
        let handle = self.handle(table)?;

        // Bounds are set on the iterator rather than checked per row, so RocksDB can skip
        // whole SST files instead of handing them up to be discarded.
        let mut read_options = ReadOptions::default();
        read_options.set_iterate_lower_bound(start.to_vec());
        read_options.set_iterate_upper_bound(end.to_vec());

        let mut rows = Vec::new();
        for row in self
            .db
            .iterator_cf_opt(handle, read_options, IteratorMode::Start)
        {
            let (key, value) = row.map_err(backend)?;
            rows.push((key.into_vec(), value.into_vec()));
        }
        Ok(rows)
    }

    fn write(&mut self, batch: WriteBatch, durability: Durability) -> Result<(), StorageError> {
        let mut rocks_batch = RocksBatch::default();
        for op in batch.ops() {
            match op {
                Op::Put { table, key, value } => {
                    rocks_batch.put_cf(self.handle(*table)?, key, value);
                }
                Op::Delete { table, key } => {
                    rocks_batch.delete_cf(self.handle(*table)?, key);
                }
                Op::DeleteRange { table, start, end } => {
                    rocks_batch.delete_range_cf(self.handle(*table)?, start, end);
                }
            }
        }

        let mut write_options = WriteOptions::default();
        write_options.set_sync(durability == Durability::Synced);
        self.db
            .write_opt(rocks_batch, &write_options)
            .map_err(backend)
    }
}
