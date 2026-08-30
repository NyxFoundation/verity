---
title: Storage Schema
last_updated: 2026-08-30
tags:
  - storage
  - rocksdb
  - schema
---

# Storage Schema

What `verity-db` persists, how it is keyed, which transitions commit together, and which short-lived
aggregation inputs remain in memory. [Architecture](../src/reference/architecture.md#storage-engine-and-retention)
settles the engine — RocksDB behind a backend trait — and this document fixes the repository layout
underneath it.

The schema is a Runtime Shell concern, not a consensus object. Consensus values are nevertheless
stored as their exact SSZ types, so a database row never invents an alternative encoding for a
protocol container.

Every size below is measured or derived from leanSpec. Measurements use the
`fixtures-prod-scheme.tar.gz` release asset; at `SECONDS_PER_SLOT = 4`, a day is 21,600 slots.

## Workloads and backend contract

| Workload | Value size | Volume | Lifetime |
|---|---:|---:|---|
| Headers, bodies, snapshots/diffs, and indices | ~100 B – 800 B | ~5 MB/day | retained |
| Aggregate block proofs (`MultiMessageAggregate`) | 155–236 KB, median 190 KB | ~4.1 GB/day before pruning | 21,600 slots |

The aggregate-proof workload selects RocksDB: its large, append-heavy values and bulk expiry fit an
LSM tree with range tombstones. The backend trait has both RocksDB and in-memory implementations and
exposes only byte operations:

- point reads;
- lexicographically ordered half-open range reads;
- atomic write batches spanning column families; and
- half-open range deletes.

The in-memory backend preserves RocksDB's lexicographic iteration order. It is used for tests and
ephemeral nodes; no RocksDB-specific behavior is allowed to leak past the trait without being named
in that contract.

RocksDB's default column family is reserved. Verity uses one application column family per logical
table so a range tombstone for proofs cannot reach another keyspace.

## Column families

`root` and `state_root` are raw 32-byte SSZ roots. `slot_be` and `validator_index_be` are fixed
8-byte unsigned big-endian integers, making lexicographic key order equal numeric order. A listed
SSZ value must decode as exactly one canonical encoding; a decode or integrity failure is storage
corruption, not an absent value.

| Column family | Key | Value | Retention / purpose |
|---|---|---|---|
| `block_headers` | `block_root` | `SSZ(BlockHeader)` | Every processed block; retained |
| `block_bodies` | `block_root` | `SSZ(BlockBody)` | Every processed block, including empty bodies; an anchor only when supplied |
| `block_proofs` | `slot_be ‖ block_root` | `SSZ(MultiMessageAggregate)` | Proof-bearing blocks; range-pruned |
| `state_snapshots` | `block_root` | `SSZ(State)` | Genesis/checkpoint anchors and periodic bases; retained |
| `state_diffs` | `block_root` | `SSZ(StateDiff)` | One per processed non-anchor block; retained |
| `canonical_blocks` | `slot_be` | `block_root` | Current canonical root at each non-empty slot |
| `state_roots` | `state_root` | `block_root` | Reverse lookup for checkpoint sync and state-root queries |
| `fork_choice_blocks` | `slot_be ‖ block_root` | `parent_root` | Processed fork-choice tree; retained |
| `known_votes` | `validator_index_be` | `SSZ(AttestationData)` | Latest counted vote per validator |
| `pending_votes` | `validator_index_be` | `SSZ(AttestationData)` | Latest not-yet-counted vote per validator |
| `metadata` | fixed ASCII key | typed scalar or SSZ value | Database identity and current view pointers |

No column family holds validator signing state. XMSS no-reuse is carried by the duty loop in
memory and nothing about it is written — see
[Key Management](key-management.md#decision-1--no-persisted-signing-state-once-per-slot-duty-dedup).

Blocks are stored unsigned: header and body are separate rows, while the signed envelope's aggregate
proof is the `block_proofs` row. `MultiMessageAggregate` is persisted as the entire SSZ container,
not merely its current `proof` field. A future container-field change therefore remains a schema
migration problem rather than silently changing the meaning of stored bytes.

`canonical_blocks` is deliberately separate from the all-branch `fork_choice_blocks` index. When the
head changes, the chain writer walks the old and new heads back to their common ancestor, deletes the
slots leaving the canonical chain, and upserts the slots joining it. Processing a side branch can
never overwrite the range-sync index.

## State storage: snapshots and diffs

The state grows with absolute slot. `process_slots` appends to `historical_block_hashes` every slot
and the current lstar state does not trim it. The 342–774 B fixture states are therefore not a
steady-state size: a state is approximately `300 B + 32 B × slot`, reaching about 691 KB at slot
21,600 and 8.4 MB at the current `HISTORICAL_ROOTS_LIMIT = 2^18`. A full state per block would grow
quadratically and cost roughly 7.5 GB over the first day alone.

Verity writes a full snapshot at genesis and checkpoint-sync anchors, then at the first processed
block whose edge from its parent crosses a 1,024-slot boundary:

```text
floor(parent_slot / 1024) < floor(block_slot / 1024)
```

Every processed branch follows this rule, so skipped slots do not lengthen replay beyond 1,023 diffs.
All non-anchor processed blocks write a `StateDiff` whose SSZ field order is:

```text
base_block_root
slot
latest_justified
latest_finalized
justified_slots
justifications_roots
justifications_validators
```

The snapshot supplies `config` and the validator registry. The reconstructor derives the latest
header and historical roots from persisted block data and the parent link. It validates the diff's
parent against the child header and validates the reconstructed state's `hash_tree_root` against the
child header's `state_root`. Snapshots and diffs are never pruned, keeping all processed state history
reconstructible.

## Metadata and identity

`metadata` accepts only the following fixed keys:

| Key | Value |
|---|---|
| `schema_version` | `SSZ(uint32)` repository schema version |
| `chain_fingerprint` | Genesis state's `hash_tree_root` (`Bytes32`) |
| `fork_version` | Canonical protocol fork-version scalar |
| `ssz_schema_digest` | `Bytes32`: SHA-256 of the versioned stored-type manifest |
| `head`, `safe_target` | `Bytes32` |
| `latest_justified`, `latest_finalized` | `SSZ(Checkpoint)` |
| `served_from_slot` | `SSZ(uint64)` |
| `last_processed_interval` | `SSZ(Interval)` |

The stored-type manifest has a fixed order: `BlockHeader`, `BlockBody`,
`MultiMessageAggregate`, `State`, `StateDiff`, `Checkpoint`, and `AttestationData`. Its digest makes
an SSZ-shape change explicit in review. The genesis state root fingerprints genesis time,
configuration, and validator registry without introducing a second chain-identity format.

A populated database opens only when all identity values match the configured network and runtime
schema. Mismatch, missing required rows, decode failure, or a root-integrity failure stops the node
and preserves the directory for diagnosis. It is never treated as an empty database or overwritten
automatically: an operator must select a new directory and explicitly checkpoint-sync again.

## Writer, batches, and restart

`verity-chain` — initially the chain module inside the single `verity-consensus` crate — owns the
only write capability, with **no exceptions**: the discipline is one writer per database, not one
writer per column family. P2P, RPC, validator duties, and maintenance submit requests to the chain
writer rather than writing the backend directly. Read-only snapshot views may run concurrently.
The rule is strong enough to express in the type system — the chain task holds the backend's only
mutable handle — so it is enforced by ownership before any model checker is pointed at it.

A processed block commits in one cross-column-family `WriteBatch`:

- header, body, and aggregate proof;
- state diff and, at an anchor, full snapshot;
- `state_roots` and `fork_choice_blocks` entries;
- canonical-index changes; and
- every metadata value affected by the transition.

Before exposing the batch, the writer checks that the header root is the block root, the body root
matches the header, the computed state root matches the header, and the parent relationships agree.
A network block whose parent or state is not yet available remains only in a bounded in-memory cache;
it is not persisted and is reacquired after restart.

Write-ahead logging is always enabled. Routine block imports and interval updates are atomic but do
not fsync. Genesis/checkpoint anchors and commits that advance finalization fsync. An anchor batch
contains its metadata, header/body when supplied, full snapshot, state-root reverse index, canonical
index, and fork-choice entry, all or none.

### Fork-choice votes and time

Raw XMSS signatures and reusable proof pools are bounded in-memory inputs for aggregation and are
discarded on restart. Fork-choice state is persisted in reduced form instead.

For every participant in a verified aggregate, the vote stored in `pending_votes` or `known_votes`
wins only if its attestation slot is newer than the old one, or on a slot tie its
`AttestationData` root is lexicographically larger. This is the deterministic LMD reduction; it
makes restart independent of proof-set insertion order.

A valid block-external aggregate updates `pending_votes`. At interval 3 the writer computes and
commits `safe_target`; at interval 4 it merges pending into `known_votes`, clears pending, and
commits the recomputed head and head-derived finalized checkpoint. Each vote-table change, affected
metadata, and `last_processed_interval` are one batch.

`last_processed_interval` is necessary because wall-clock time alone cannot reveal which interval
events completed before a crash. Restart validates the rows, reconstructs the fork-choice tree from
`latest_justified.slot` onward with a range scan, recomputes head/safe-target/checkpoint metadata
from the two vote maps, checks exact agreement with metadata, then replays ticks through the current
wall-clock interval. Any disagreement fails closed.

## Retention and range sync

Only proofs and logically stale votes are deleted.

**Block proofs.** Let `cutoff = tip_slot − 21,600`, saturating at zero. When
`cutoff ≤ latest_finalized.slot`, maintenance deletes `[0, cutoff)` from `block_proofs` in one range
tombstone. Otherwise it does nothing. This never removes a proof from the current non-finalized range.
Headers, bodies, snapshots, diffs, state-root mappings, and fork-choice blocks are retained.

**Votes.** The compact vote maps discard an entry only when current fork-choice rules declare it
irrelevant: its head is at or below the finalized slot, or its head is not a descendant of the
finalized block. Blocks themselves remain available even after their votes lose relevance.

leanSpec sets `MIN_SLOTS_FOR_BLOCK_REQUESTS = 3600` (four hours), which a `BlocksByRange` responder
must serve. The proof window is six times that floor, allowing a node down overnight to rejoin with
range sync rather than a checkpoint.

A checkpoint-sync node does not advertise range service until it has fetched and verified a
proof-bearing canonical history covering the required recent window. `served_from_slot` records the
first proved slot; requests below `max(current_slot − 3600, served_from_slot)` return
`RESOURCE_UNAVAILABLE`. Inside that advertised window, a missing proof for a canonical non-anchor
block is storage corruption, not an empty slot. Genesis and checkpoint-sync anchors without their
original signed proof are omitted from `BlocksByRange`; no proof is synthesized.

## Physical tuning

`block_proofs` is uncompressed: aggregate proofs are high-entropy cryptographic blobs and gain
little from compression. All other column families use LZ4. Cache size, memtable size, compaction
parallelism, and related RocksDB settings are operational tunables set from production-fixture
benchmarks, never consensus constants.

## In-memory only

| Held | Bound | Lost on restart |
|---|---|---|
| Raw gossip XMSS signatures awaiting aggregation | entry count | yes |
| Reusable pending and known aggregate-proof pools | entry count | yes |
| Pending blocks awaiting a parent or state | entry count | yes |
| Reconstructed state cache | LRU | yes |
