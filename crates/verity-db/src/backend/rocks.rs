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

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rocksdb::{
    ColumnFamilyDescriptor, DB, DBCompressionType, IteratorMode, Options, ReadOptions,
    WriteBatch as RocksBatch, WriteOptions,
};

use crate::column::ColumnFamily;
use crate::error::StorageError;

use super::{Durability, Op, Rows, StorageBackend, StorageReader, WriteBatch};

/// A read-only handle on an open database, shareable across tasks.
///
/// RocksDB takes an exclusive lock on its directory, so a second reader cannot simply open
/// the same path: it has to share the handle the writer opened. `DB` is `Sync` and its reads
/// take `&self`, so sharing one behind an `Arc` is the engine's own supported arrangement,
/// and a read here is consistent with whatever the writer has already committed.
///
/// The type carries no `write`, which is what keeps `docs/design/storage.md`'s one-writer
/// rule true across tasks rather than only across owners.
#[derive(Debug, Clone)]
pub struct RocksReader {
    db: Arc<DB>,
    path: PathBuf,
}

impl RocksReader {
    /// The directory this database lives in.
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

/// A RocksDB database with one column family per logical table.
#[derive(Debug)]
pub struct RocksBackend {
    reader: RocksReader,
}

impl RocksBackend {
    /// Opens a database, creating the complete schema only in a new, empty directory.
    ///
    /// An existing database must contain exactly the schema's column families. Missing tables
    /// are corruption rather than a migration request, so opening never recreates them.
    ///
    /// # Errors
    ///
    /// - [`StorageError::ColumnFamilySet`] when an existing directory has a different schema.
    /// - [`StorageError::Backend`] when RocksDB cannot inspect or open the directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();

        let existing = existing_column_families(&path)?;
        if let Some(column_families) = &existing {
            validate_column_families(column_families)?;
        }

        let is_new = existing.is_none();
        let mut db_options = Options::default();
        db_options.create_if_missing(is_new);
        db_options.create_missing_column_families(is_new);

        let descriptors = ColumnFamily::ALL
            .map(|table| ColumnFamilyDescriptor::new(table.name(), table_options(table)));

        let db = DB::open_cf_descriptors(&db_options, &path, descriptors).map_err(backend)?;
        Ok(Self {
            reader: RocksReader {
                db: Arc::new(db),
                path,
            },
        })
    }

    /// The directory this database lives in.
    ///
    /// A failure preserves the directory for diagnosis rather than repairing it, so the
    /// operator has to be told where to look.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.reader.path()
    }

    /// A read-only handle on the same open database, for a task that must not write.
    #[must_use]
    pub fn reader(&self) -> RocksReader {
        self.reader.clone()
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

fn existing_column_families(path: &Path) -> Result<Option<Vec<String>>, StorageError> {
    match DB::list_cf(&Options::default(), path) {
        Ok(column_families) => Ok(Some(column_families)),
        Err(error) => {
            if directory_is_empty(path)? {
                return Ok(None);
            }
            Err(backend(error))
        }
    }
}

fn directory_is_empty(path: &Path) -> Result<bool, StorageError> {
    let mut entries = match path.read_dir() {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(StorageError::Backend(error.to_string())),
    };
    entries
        .next()
        .transpose()
        .map(|entry| entry.is_none())
        .map_err(|error| StorageError::Backend(error.to_string()))
}

fn validate_column_families(actual: &[String]) -> Result<(), StorageError> {
    let expected: BTreeSet<String> = std::iter::once("default".to_string())
        .chain(
            ColumnFamily::ALL
                .into_iter()
                .map(|table| table.name().to_string()),
        )
        .collect();
    let actual: BTreeSet<String> = actual.iter().cloned().collect();
    let missing: Vec<String> = expected.difference(&actual).cloned().collect();
    let unexpected: Vec<String> = actual.difference(&expected).cloned().collect();
    if missing.is_empty() && unexpected.is_empty() {
        return Ok(());
    }
    Err(StorageError::ColumnFamilySet {
        missing,
        unexpected,
    })
}

fn backend(error: rocksdb::Error) -> StorageError {
    StorageError::Backend(error.to_string())
}

impl StorageReader for RocksReader {
    fn get(&self, table: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        self.db.get_cf(self.handle(table)?, key).map_err(backend)
    }

    fn table_is_empty(&self, table: ColumnFamily) -> Result<bool, StorageError> {
        let mut rows = self
            .db
            .iterator_cf(self.handle(table)?, IteratorMode::Start);
        match rows.next() {
            None => Ok(true),
            Some(row) => row.map(|_| false).map_err(backend),
        }
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
}

impl StorageReader for RocksBackend {
    fn get(&self, table: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        self.reader.get(table, key)
    }

    fn table_is_empty(&self, table: ColumnFamily) -> Result<bool, StorageError> {
        self.reader.table_is_empty(table)
    }

    fn range(&self, table: ColumnFamily, start: &[u8], end: &[u8]) -> Result<Rows, StorageError> {
        self.reader.range(table, start, end)
    }
}

impl StorageBackend for RocksBackend {
    fn write(&mut self, batch: WriteBatch, durability: Durability) -> Result<(), StorageError> {
        let mut rocks_batch = RocksBatch::default();
        for op in batch.ops() {
            match op {
                Op::Put { table, key, value } => {
                    rocks_batch.put_cf(self.reader.handle(*table)?, key, value);
                }
                Op::Delete { table, key } => {
                    rocks_batch.delete_cf(self.reader.handle(*table)?, key);
                }
                Op::DeleteRange { table, start, end } => {
                    rocks_batch.delete_range_cf(self.reader.handle(*table)?, start, end);
                }
            }
        }

        let mut write_options = WriteOptions::default();
        write_options.set_sync(match durability {
            Durability::Synced => true,
            Durability::Buffered => false,
        });
        self.reader
            .db
            .write_opt(rocks_batch, &write_options)
            .map_err(backend)
    }
}
