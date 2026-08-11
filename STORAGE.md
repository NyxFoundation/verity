# Storage Schema

What `verity-db` persists, how it is keyed, and what is deliberately kept in memory instead.
[Architecture](ARCHITECTURE.md#storage-engine-and-retention) settles the engine (RocksDB behind a
backend trait) and the aggregate-proof retention window; this document is the layout underneath
those decisions.

Every figure here is measured or derived from leanSpec, not estimated. The measurements come from
the `fixtures-prod-scheme.tar.gz` release asset; the growth model comes from the state transition
itself. At `SECONDS_PER_SLOT = 4` a day is 21,600 slots.

## What is persisted, and what is not

leanSpec's fork-choice `Store` and the set a node actually writes to disk are not the same thing.
The reference node's `Database` protocol (`src/lean_spec/node/storage/database.py`) persists a
strict subset:

| `Store` field | Persisted |
|---|---|
| `blocks: root → Block` (unsigned) | yes |
| `states: root → State` | yes |
| `head`, `safe_target`, `latest_justified`, `latest_finalized`, `time`, `config` | yes, as scalars |
| `attestation_signatures` (raw XMSS, 2,536 B each) | **no** — memory only |
| `latest_new_aggregated_payloads`, `latest_known_aggregated_payloads` | **no** — memory only |

Verity follows that split, with one deliberate addition: aggregate **block** proofs are persisted,
because a restart otherwise leaves the node unable to serve `BlocksByRange` until fresh blocks
arrive. The attestation-level buffers stay in memory, bounded by entry count.

Blocks are stored **unsigned**. The proof is a separate row, so a block whose proof has been pruned
still reads back as a block.

## The size model

Two facts shape every table below.

**Aggregate proofs dominate write volume.** 155–236 KB each (median 190 KB) against ~100–800 B for
everything else: ~4.1 GB/day of proofs versus ~5 MB/day of blocks.

**State grows without bound, one slot at a time.** The 342–774 B seen in SSZ fixtures is not a
steady-state size. `process_slots` appends to `historical_block_hashes` every slot and never trims
it — it cannot, because the list is indexed by absolute slot:

```python
new_historical_block_hashes = (
    state.historical_block_hashes + [parent_root] + [ZERO_HASH] * num_empty_slots
)
```

The justification fields do not share this problem: `justified_slots` and `justifications_*` are
indexed relative to the finalized boundary, so they stay bounded by the non-finalized window.

State size is therefore `≈ 300 B + 32 B × slot`, capped only by `HISTORICAL_ROOTS_LIMIT = 2^18`:

| Point on the chain | One state |
|---|---|
| slot 21,600 (one day) | ~691 KB |
| slot 262,144 (~12.1 days, the SSZ limit) | ~8.4 MB |

Writing a full state per block would cost ~7.5 GB over the first day alone (`Σ 32·s`) and grow
quadratically — heavier than the proofs. Snapshots plus diffs bring the same data to ~10–15 MB/day.
That is why the state tables are split, and why a diff must not carry
`historical_block_hashes`.

## Tables

| Table | Key | Value | Pruned |
|---|---|---|---|
| `Blocks` | root | `Block` (unsigned) | never |
| `BlockProofs` | slot ‖ root | `MultiMessageAggregate` | below `tip − 21,600`, once finalized |
| `BlockRoots` | slot | canonical block root | never |
| `StateSnapshots` | root | full `State` | never |
| `StateDiffs` | root | `StateDiff` | never |
| `StateRootIndex` | state_root | block root | never (tied to its block) |
| `LiveChain` | slot ‖ root | parent_root | below finalized |
| `Metadata` | string | SSZ scalars | never |

`Metadata` holds the `Store`'s own persistent scalars — `time`, `config`, `head`, `safe_target`,
`latest_justified`, `latest_finalized`. A network fingerprint is derived from `genesis_time` (a
field of `config`), so a data directory belonging to another network is refused at startup rather
than silently resumed.

`LiveChain` is an index, not a copy: it lets fork choice build the `root → (slot, parent_root)`
tree without deserializing a block. Presence in it is also what makes a block *visible* to fork
choice, which gives pending blocks a natural representation — a block waiting for its parent has
its rows written, including the heavy proof, but no `LiveChain` entry until it is processed.

`BlockRoots` maps the canonical chain only. A reorg rewrites the affected slot range by walking the
old and new head back to their common ancestor, so the cost tracks the reorg depth rather than the
chain length.

### Key encoding

- **Root-keyed** tables use the 32-byte root.
- **Slot-prefixed** tables (`BlockProofs`, `LiveChain`) use an 8-byte **big-endian** slot followed
  by the 32-byte root. Big-endian makes lexicographic order equal numeric slot order, which is what
  turns pruning into a range delete instead of a scan. This is the same property that selected an
  LSM engine.
- **Slot-keyed** `BlockRoots` uses the 8-byte big-endian slot alone; the value already holds the root.

One RocksDB column family per table. Beyond isolation, this is required for correctness of the
proof pruning: a range delete must not be able to reach another table's keyspace.

## State storage: snapshots and diffs

Writing a state does two things:

1. **Always** a `StateDiff` keyed by the block root, linked to its parent by `base_root`.
2. **At anchors only** a full snapshot, when a block crosses a 1,024-slot boundary relative to its
   parent (~68 minutes). This bounds any reconstruction to at most 1,024 diff applications.

A diff carries only what cannot be recovered elsewhere: the target slot, the justified and
finalized checkpoints, and the justification fields in full (bounded by the non-finalized window).
Everything else is omitted and recovered:

| Omitted | Recovered from |
|---|---|
| `config`, `validators` | the snapshot — fixed at genesis; under the current lstar STF neither is mutated after genesis |
| `latest_block_header` | `Blocks` |
| `historical_block_hashes` | regenerated from `base_root` plus the slot gap |

That last row is the one that makes the scheme work. Regeneration is checked rather than trusted:
an append whose hashes do not match the expected slot gap, or which is not zero-filled across
skipped slots, is rejected where the diff is created, so a bad append cannot surface later as a
corrupted reconstruction.

The `config` and `validators` row is scoped to the current fork: the lstar STF mutates neither
after genesis, so a snapshot carries the live set and a diff need not. A future fork that activates
or slashes validators — as mainnet consensus does — would have to carry those mutations in the
diff; that is the point to revisit if this scheme is reused.

Reads resolve in three steps — an in-memory LRU of recent states, then a snapshot, then
reconstruction by walking `base_root` back to the nearest snapshot and replaying forward. States
are content-addressed and immutable, so the cache never needs invalidation.

Snapshots and diffs are **never pruned**: the full state history stays reconstructible. The cost is
a monotonically growing diff chain, accepted for the debugging and verification leverage it gives
against a spec that is still moving.

## Pruning

Only two paths delete anything.

**Block proofs.** With `cutoff = tip_slot − 21,600`: if `cutoff` is at or below the finalized slot,
delete every proof below `cutoff` in a single range delete; otherwise delete nothing. A non-finalized
proof is never touched, and a chain that has stopped finalizing simply stops pruning. Blocks and
states are untouched by this path — only the proof is removed, so history stays intact.

The floor is fixed upstream: leanSpec sets `MIN_SLOTS_FOR_BLOCK_REQUESTS = 3600` (4 hours) and a
`BlocksByRange` responder MUST serve that window. 21,600 slots is an operational choice 6× above it
— see [Architecture](ARCHITECTURE.md#storage-engine-and-retention).

**`LiveChain`.** Pruned below the finalized slot, keeping the finalized block itself as the anchor
for fork choice.

## In memory only

| Held | Bound | Lost on restart |
|---|---|---|
| Raw gossip XMSS signatures awaiting aggregation | entry count | yes |
| New (pending) aggregated payloads | entry count | yes |
| Known (fork-choice-active) aggregated payloads | entry count | yes |
| Reconstructed state cache | LRU | yes |

None of these is needed to rebuild the chain: they are working set for the current and next few
slots. leanSpec's reference node holds them the same way.
