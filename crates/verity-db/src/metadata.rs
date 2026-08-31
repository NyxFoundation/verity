//! The fixed key set of the `metadata` table.
//!
//! `metadata` is the one table without a structural key, so the set of keys it accepts is
//! closed here instead. Four of them identify the database; the rest are the current view the
//! node restarts from.
//!
//! Transcribed from `docs/design/storage.md`, "Metadata and identity".

use core::fmt;

/// A key the `metadata` table accepts. No other key may be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataKey {
    /// `SSZ(uint32)` repository schema version.
    SchemaVersion,
    /// The genesis state's `hash_tree_root`, as `Bytes32`.
    ChainFingerprint,
    /// The canonical protocol fork-version scalar, as `SSZ(uint64)`.
    ForkVersion,
    /// SHA-256 of the versioned stored-type manifest, as `Bytes32`.
    SszSchemaDigest,
    /// The current head block root, as `Bytes32`.
    Head,
    /// The current safe target block root, as `Bytes32`.
    SafeTarget,
    /// `SSZ(Checkpoint)`.
    LatestJustified,
    /// `SSZ(Checkpoint)`.
    LatestFinalized,
    /// `SSZ(uint64)`: the first slot range sync may be served from.
    ServedFromSlot,
    /// `SSZ(Interval)`: the last interval whose events fully committed.
    LastProcessedInterval,
}

impl MetadataKey {
    /// The four values that identify which chain, fork, and schema this database holds.
    ///
    /// They are written once, by the anchor commit, and are never updated afterwards. A
    /// populated database must carry all four or it does not open.
    pub const IDENTITY: [Self; 4] = [
        Self::SchemaVersion,
        Self::ChainFingerprint,
        Self::ForkVersion,
        Self::SszSchemaDigest,
    ];

    /// The view pointers a restart reconstructs and re-checks.
    pub const VIEW: [Self; 6] = [
        Self::Head,
        Self::SafeTarget,
        Self::LatestJustified,
        Self::LatestFinalized,
        Self::ServedFromSlot,
        Self::LastProcessedInterval,
    ];

    /// The literal bytes of the key as stored.
    ///
    /// These strings are part of the on-disk format; changing one is a schema migration.
    #[must_use]
    pub const fn as_bytes(self) -> &'static [u8] {
        match self {
            Self::SchemaVersion => b"schema_version",
            Self::ChainFingerprint => b"chain_fingerprint",
            Self::ForkVersion => b"fork_version",
            Self::SszSchemaDigest => b"ssz_schema_digest",
            Self::Head => b"head",
            Self::SafeTarget => b"safe_target",
            Self::LatestJustified => b"latest_justified",
            Self::LatestFinalized => b"latest_finalized",
            Self::ServedFromSlot => b"served_from_slot",
            Self::LastProcessedInterval => b"last_processed_interval",
        }
    }
}

impl fmt::Display for MetadataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Every key is ASCII by construction, so this cannot lose information.
        f.write_str(core::str::from_utf8(self.as_bytes()).unwrap_or("<non-utf8 metadata key>"))
    }
}

#[cfg(test)]
mod tests {
    use super::MetadataKey;

    const ALL: [MetadataKey; 10] = [
        MetadataKey::SchemaVersion,
        MetadataKey::ChainFingerprint,
        MetadataKey::ForkVersion,
        MetadataKey::SszSchemaDigest,
        MetadataKey::Head,
        MetadataKey::SafeTarget,
        MetadataKey::LatestJustified,
        MetadataKey::LatestFinalized,
        MetadataKey::ServedFromSlot,
        MetadataKey::LastProcessedInterval,
    ];

    #[test]
    fn should_partition_every_key_into_identity_or_view() {
        for key in ALL {
            let identity = MetadataKey::IDENTITY.contains(&key);
            let view = MetadataKey::VIEW.contains(&key);
            assert!(identity ^ view, "{key} belongs to exactly one group");
        }
    }

    #[test]
    fn should_give_every_key_a_distinct_ascii_name() {
        let mut names: Vec<_> = ALL.iter().map(|key| key.as_bytes()).collect();
        assert!(names.iter().all(|name| name.is_ascii()));
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }
}
