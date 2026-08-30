---
title: Validator Key Management — XMSS Signing State
last_updated: 2026-08-30
tags:
  - xmss
  - key-management
  - validator
---

# Validator Key Management — XMSS Signing State

> Status: pre-implementation. Decisions ratified 2026-08-16; Decision 1 re-decided 2026-08-30.
> This document settles how Verity manages its validators' XMSS keys: the no-reuse guarantee,
> key material loading, and preparation scheduling. It adds no column family to
> [storage.md](storage.md) and plugs into the runtime model of
> [concurrency.md](concurrency.md).

The stake is unusual and worth sizing precisely first, because an earlier revision of this
document oversized it and reached a heavier decision as a result. XMSS is a **stateful
one-time signature scheme**: signing two *different* messages with the same key at the same
epoch does not incur a protocol penalty — the lean protocol has no slashing at all — it
degrades the *key material itself*, which no penalty schedule can undo.

**The blast radius is one epoch, not the key.** leanSig derives each epoch's one-time chain
start values from the PRF applied to that epoch:
`PRF::get_domain_element(&sk.prf_key, epoch, index)` in `sign`
(`src/signature/generalized_xmss.rs`). Epochs are therefore independent, and the Merkle tree
above them authenticates per-epoch public keys. Two different messages at one epoch expose
that epoch's chain values and nothing else: the top tree, the PRF seed, and every other epoch
stay intact. At `LOG_LIFETIME = 32` the key remains usable for its other ~4.29 billion slots.

What an attacker gains is therefore bounded: the ability to forge **one validator's message at
one slot**, subject to finding a message whose incomparable encoding is dominated by the
revealed chain positions. The protocol already tolerates equivocation — fork choice accepts
two blocks from one proposer at one slot, and no slashing container exists — so the forged
message is worth one validator's weight at one already-past slot. Everything below is sized
against that, and not against a whole-key compromise.

## What the spec and the library fix

Read at leanSpec `main` = `0588c2d2`, leanSig `main` = `c08a3ba`.

**leanSpec** (`src/lean_spec/spec/crypto/xmss/`, `.../lstar/containers/validator.py`):

- **Epoch is the slot, exactly.** `sign()` and `verify()` take the consensus `Slot` and pass it
  unmodified as the XMSS epoch index. No offset, no mapping.
- **Two independent keys per validator** — `attestation_public_key` and `proposal_public_key`
  in the registry entry. A proposer signs a block *and* an attestation in its slot; one
  one-time key cannot cover both, so the spec separates the roles at the key level and the
  reference node refuses to load identical keys for both roles.
- **Signatures are deterministic** in `(secret_key, slot, message)`: re-signing the *same*
  message at the same slot yields the identical signature and is harmless. The prohibition is
  precisely: *never two different messages for the same (key, slot)*.
- **Key lifetime is a non-issue**: production `LOG_LIFETIME = 32` → 2³² slots ≈ 544 years at
  4-second slots. There is no key-rotation mechanism in the spec, and none is needed.
- **The protocol tolerates equivocation**: fork choice accepts two blocks from one proposer at
  one slot; no slashing container exists.
- **The reference node is not state-free — it persists, and deliberately excludes signing
  state.** `src/lean_spec/node/storage/` is a real SQLite backend whose tables are `blocks`,
  `states`, `checkpoints` (justified, finalized, head, and `genesis_time`, the last one
  commented "Persisted so the node can restart without external genesis config"),
  `slot_index`, and `state_root_index`. Restart is a designed-for scenario there. No table
  holds signing state. The absence is a choice made *with* a durability layer present, not the
  by-product of not having one.

**leanSig** (Rust, the library `verity-crypto` wraps):

- `sign(sk: &SecretKey, epoch: u32, msg)` takes an **immutable** reference and has **no
  anti-reuse guard whatsoever** — same `(sk, epoch)` with a second message silently produces
  the forgery catastrophe. The README makes non-reuse entirely the caller's responsibility.
- `sign` **panics** (`assert!`) when the epoch is outside the key's activation interval or its
  *prepared interval* — it does not return an error.
- The ~33.5 MB secret key holds a top Merkle tree plus a sliding window of **two bottom
  trees**, each covering 2¹⁶ = 65,536 epochs. The prepared (signable) window is therefore
  131,072 slots ≈ **6 days**. `advance_preparation(&mut self)` slides the window one bottom
  tree (≈ 3 days) forward by rebuilding 65,536 one-time chains — a genuinely expensive,
  rayon-parallel computation. The window state is derived: it can always be recomputed from
  the PRF seed, at the cost of one rebuild per 3-day step since the key's activation.
- Canonical serialization is hand-written SSZ (`ethereum_ssz`). **No loader from the
  ecosystem's key-file formats exists in any repository** — leanSig ships none, and
  `leansig-test-keys` is data only (8 production-scheme keypairs, attestation role only,
  JSON-hex envelopes produced by leanSpec's Python tooling).

## Survey — what the reference node and other clients do

Read at leanSpec `0588c2d2`, ethlambda `e16f477`, ream `842a709`, zeam `6495beb`,
qlean-mini `55b6eb3`.

| Client | Persisted signing state | Anti-double-sign guard | `advance_preparation` |
|---|---|---|---|
| leanSpec (reference) | none (its SQLite backend has no such table) | in-memory `_attested_slots` set, 4-slot retention, attestations only | inline in `_sign_with_key`, advancing until the slot is prepared |
| ethlambda | none | none | per-slot pre-advance on the chain actor + startup catch-up |
| ream | none | in-memory block-prebuild flag only | **never called in production code** |
| zeam | none | none; **falls back to one key for both roles** when only one file is present | lazily, inline inside `sign`, under the signing mutex |
| qlean-mini | none | none | none |

No implementation in the ecosystem, the reference node included, persists signing state. The
survey also produced two live counterexamples of what unmanaged key state looks like: ream will
panic at the prepared-window boundary because nothing ever advances it, and zeam's single-key
fallback reproduces exactly the dual-role reuse the reference loader rejects
(`node/validator/registry.py`: "Sharing one key across both roles reuses one-time state and
breaks it").

### What `_attested_slots` actually is

It is duty-loop idempotence, not a key-safety mechanism, and the duty loop's own branch
conditions show why (`node/validator/service.py`):

```python
if interval == Interval(0):                 # block production: structurally reached once per slot
    ...
if interval >= Interval(1) and slot not in self._attested_slots:   # attestation: reachable at 1,2,3,4
    ...
```

Block production sits behind `interval == 0`, so one slot admits one attempt and no set is
needed. Attestation sits behind `interval >= 1`, so one slot admits up to four passes — the set
collapses them to one. `ATTESTED_SLOT_RETENTION = 4` is documented as "Slots of attestation
dedup history to keep; older slots can no longer be attested", which is a statement about what
the duty loop will ever revisit, not about key lifetime.

### The reference node's actual answer to a rewinding clock

leanSpec does not assume the wall clock moves forward. It says so, names the causes, and
defends the consensus side (`node/chain/service.py`):

```python
# The target comes from the wall clock, which can step backward.
# NTP slew, a leap second, or a VM migration can move it before the store time.
# A backward target would tick nothing, so return early.
if target_interval <= store.time:
    return []
```

The defence is that **`store.time` is monotone**: a rewound wall clock cannot rewind consensus
processing. That monotonicity is not extended to the validator duty path, which reads
`self.clock.current_slot()` directly off a plain wall clock (`node/chain/clock.py` imports
`from time import time as wall_time`; there is no monotonic source anywhere). The rewind
magnitude the reference node is sized for can be read off the causes it names: an NTP
correction and a leap second are sub-second to a few seconds, and the 4-slot (16-second)
attestation window covers exactly that band. Only "a VM migration" can exceed it, and that case
is named but not sized for.

### Scenarios, restated at their real weight

1. **Intra-slot crash-restart** — the process restarts inside one 4-second slot and re-signs it
   with a changed view. **Unreachable in Verity.** Startup loads 33.5 MB × 2 roles ≈ 67 MB of
   key material per validator (Decision 2) before any duty runs, and may owe preparation
   advances on top (Decision 3). A restart does not complete inside one slot.
2. **In-process clock step-back** — the derived slot rewinds while the process runs. Real, but
   **bounded small in normal operation**: a backward step requires the OS clock to have been
   *ahead* of true time, and a continuously running NTP daemon slews rather than steps once the
   offset is small, so the offset never grows to slot scale. Exceeding 4 slots takes a
   precondition — NTP absent for a long period, an RTC set ahead, or a manual `date -s`.
3. **State restored underneath a running or restarting node** — a datadir or VM snapshot
   restored from an earlier point, or two instances started on one key set. A snapshot restore
   rolls process memory back too, so no in-memory mechanism can observe it; two instances share
   nothing to coordinate through. **Out of scope of any local mechanism; operator
   responsibility**, stated in the operator documentation.

## Decision 1 — no persisted signing state; once-per-slot duty dedup

Verity persists **nothing** about signing. The no-reuse guarantee is the same shape the
reference node uses: an **in-memory, once-per-slot duty dedup**, and a duty loop whose
structure is what makes that dedup sufficient.

- **Duty loop structure, mirroring leanSpec.** Block production runs only at interval 0.
  Attestation runs at interval ≥ 1 and may be reached on any later interval of the same slot,
  so a slow proposal does not cost that slot's attestation. This is leanSpec's structure and
  Verity adopts it unchanged; `verity-chain`'s `SlotClock::current_slot(&self,
  now_milliseconds: u64)` already takes its time as an argument rather than reading an ambient
  wall clock, matching the reference node's injectable `time_fn`.
- **The dedup follows from that structure.** Attestation carries an in-memory set of
  already-attested slots with 4-slot retention, because interval ≥ 1 admits up to four passes
  per slot. Block production carries none, because interval 0 admits one. This is a
  consequence of the loop shape, not an independent judgement about the two roles — if the
  loop shape changes, this changes with it.
- **Nothing is written, so nothing is fsynced.** The signing path performs no I/O.
- **Startup has no signing-state gate.** Duties are gated by key preparation, the first
  `ChainView`, and sync state (Decision 3), never by signing history.

### Why not persist a watermark

An earlier revision of this document specified a persisted per-`(validator, role)` watermark in
a `signing_watermarks` column family, fsynced before each signature was released. It is
withdrawn. Three findings, in the order they bind:

1. **The threat it priced does not exist at that size.** The decision was sized against
   whole-key compromise. The blast radius is one epoch (see the opening section) — one
   validator's forgeable message at one slot, in a protocol with no slashing that already
   tolerates equivocation.
2. **Its runtime cost lands in the worst possible place.** A durable write before signature
   release puts an fsync on the signing path, sharing the storage engine's WAL with the
   aggregate-proof workload — 155–236 KB per proof, median 190 KB, ~4.1 GB/day
   ([storage.md](storage.md)). Signature-release latency then becomes a function of compaction
   and flush tail behavior, inside the interval budget the attestation must meet. It converts
   a stable, I/O-free path into one coupled to the least predictable component in the node.
3. **Its failure mode is unrecoverable by the operator's natural action.** The design failed
   closed at startup whenever `watermark ≥ current_slot`, disabling that validator's duties
   until the clock passed the watermark. Because the state was durable, a restart did not clear
   it. That trades a *certain* availability loss, of unbounded duration, against the
   probability of a *bounded* one-slot forgeability — and it does so in the scenario class
   (scenario 3 above) it cannot actually cover, since a snapshot restore rolls the watermark back with
   everything else.

The residual exposure is stated plainly: an in-process clock step-back exceeding 4 slots would
let a validator re-sign a slot it has already signed. Scenario 2 above bounds how that arises;
the reference node accepts the same exposure with the same 4-slot window. Verity accepts it,
and does not buy it off with an fsync on the signing path.

### What carries the guarantee instead

- **Role separation**, enforced at load time: distinct attestation and proposal keys, identical
  keys rejected (Decision 2). This is what keeps a proposer's block signature and its
  attestation in the same slot from colliding.
- **Duty-loop structure plus the dedup set**, above: one signature per `(validator, role,
  slot)`, forward only.
- **Determinism as a backstop.** leanSig's randomness comes from
  `PRF::get_randomness(&sk.prf_key, epoch, message, attempts)`, so re-signing the *same*
  message at the same slot reproduces the identical signature and is harmless. The prohibition
  is precisely two *different* messages at one `(key, slot)`; an idempotent retry is not one.

## Decision 2 — key material: follow the de-facto standard, harden the loader

The ecosystem has a de-facto standard Verity does not deviate from: lean-quickstart's genesis
generator emits `validator-config.yaml`, `annotated_validators.yaml`, and a `hash-sig-keys/`
directory of leanSig-SSZ secret-key files, and all four surveyed clients consume variants of
it. Interop with that tooling is the entire point of key loading, so:

- **Format and layout: lean-quickstart compatible.** leanSig-SSZ key files under
  `hash-sig-keys/`, indexed by the YAML manifest. No encrypted keystore (no EIP-2335
  analogue): devnet keys are disposable and no ecosystem precedent exists; revisit only when
  a persistent network makes keys worth stealing.
- **The loader is Verity's own, in `verity-crypto`** — none exists anywhere to reuse. The
  manifest's role declarations are authoritative; file-name substring inference (the
  ethlambda/zeam convention) is not used.
- **Startup rejections, fail-closed.** Missing either role's key, identical attestation and
  proposal keys (zeam's fallback is explicitly the counterexample), or a loaded public key
  that does not match the manifest ⇒ refuse to start. These are the reference loader's checks,
  kept.
- **Fully memory-resident**, like every surveyed client: production scheme ≈ 33.5 MB × 2
  roles ≈ **67 MB per validator** (64 validators ≈ 4.3 GB). That linear bound is recorded
  here as fact; mmap or partial loading is not built until a measured need exists.
- **CI note.** `leansig-test-keys` provides 8 production-scheme keypairs for the attestation
  role only; proposal-role keys for tests come from lean-quickstart's generator.

## Decision 3 — preparation scheduling

The prepared window is 6 days wide and costs one heavy rebuild per 3-day step; ream (never
advances → future panic) and zeam (advances inside `sign` → the signing call stalls at the
window boundary) mark the two failure modes to design out.

- **Steady state.** Each validator-duty tick performs a cheap comparison: has the current
  slot passed the midpoint of this key's prepared interval? If so, an advance is started on
  `spawn_blocking` — leanSig's own intended cadence (≈ every 3 days per key, with ≈ 3 days of
  margin).
- **Clone-advance-swap: signing never waits.** `advance_preparation` needs `&mut`; handing
  the live key to a worker (or locking it) would stall signing for the rebuild duration. So
  the worker receives a **copy**, advances it off-thread while
  the original keeps signing, and the validator task swaps the advanced copy in between
  sign calls. The task is single-threaded, so the swap is a plain field replacement with no
  torn state. **The copy is not a memcpy.** leanSig's secret key does not implement `Clone`,
  so `verity-crypto`'s `SecretKey::duplicate` goes through the canonical encoding: about
  33.5 MB serialized and parsed again. Still far cheaper than the rebuild it runs alongside,
  and paid about once every three days per key, but it is not the pointer-width copy the
  shape suggests. **Why the original stays valid throughout:** the windows overlap. Advancing at
  the midpoint means the current slot sits in the old window's *second* bottom tree; the
  advance produces a window of that same second tree plus the next one. Old and new windows
  therefore share ≈ 3 days of coverage around the current slot — there is no moment at which
  either key object is unable to sign for "now", however long the rebuild takes within that
  margin. Both key objects are the same key; no-reuse is enforced by the duty loop's
  once-per-slot dedup, independent of which copy signs.
- **Persist the advanced key.** The `spawn_blocking` worker itself, as the final step of the
  advance job and *before* returning the advanced clone, rewrites the key file atomically
  (temp file + rename; public key verified against the manifest on load) — so a key the
  validator task swaps in is already durable. A failed file write is logged and is
  **non-fatal**: the swap still proceeds, because persisting the advanced key only bounds
  restart cost — it carries no part of the no-reuse guarantee; a stale file costs extra
  catch-up and nothing else. No surveyed client persists advanced keys, and the omission is
  why a year-old node would owe ~120 consecutive rebuilds (minutes to hours) at startup: the
  on-disk key never moves off its activation window. With periodic persistence, startup
  catch-up is bounded by the downtime, not the node's age.
- **Panic containment.** leanSig's `sign` asserts; Runtime Shell code must not panic. The
  `verity-crypto` wrapper checks the activation and prepared intervals first and returns a
  typed error — the assert becomes unreachable, and Kani's target is exactly that
  unreachability.
- **Startup order — who runs it, and when.** Key preparation is `verity-validator`'s own
  initialization: the `verity` binary **spawns the validator task at process start, alongside
  the chain task**, handing it the key directory at construction. The task then loads keys and
  advances each key on `spawn_blocking` until the current slot is inside its prepared interval (which
  may take a while after long downtime — progress is logged). None of this needs consensus
  state, so it runs in parallel with the chain task's own startup — per
  [concurrency.md](concurrency.md#lifecycle), what the first `ChainView` gates is *serving*,
  not construction. The duty loop begins serving only when **every serving gate** is open — a join of
  independent conditions, of which this document contributes two: the first `ChainView` has
  been observed on its `watch` receiver (the concurrency.md readiness signal, unchanged)
  *and* key preparation has completed. [sync.md](sync.md#decision-1--sync-mode-lifecycle)
  adds the third: the node's sync state is `SYNCED`.

## Deliberately out of scope

- **Remote signers** and any signing API surface.
- **Key generation ceremony / tooling** — devnet keys come from lean-quickstart's generator;
  Verity ships no keygen.
- **Hot key reload** — key set changes require a restart.
- **A separate slashing-protection database** — the protocol has no slashing, and an
  eth2-style interchange format has no counterpart here. More generally, **no signing state is
  persisted at all** (Decision 1).
- **Defending state restored underneath the node** — a datadir or VM snapshot rolled back to an
  earlier point, or two instances sharing one key set (scenario 3). No local mechanism can
  observe either: a snapshot restore rolls back process memory along with everything else, and
  two instances share nothing to coordinate through. This is an operator responsibility and
  belongs in the operator documentation, not in the node.

## Verification obligations introduced by this model

- **The once-per-slot property** is the invariant of this document: *the duty loop releases at
  most one signature per `(validator, role, slot)`, and slots are visited in increasing order*.
  It is a sequential property of a single task's loop, targeted with property tests over
  interval sequences — no crash injection, because nothing is written, and no Loom, because no
  concurrency is involved.
- **Kani / bolero:** the loader (no-panic-on-any-input over manifest and key bytes; reject ⇒
  no partial registry) and the sign wrapper (leanSig's asserts unreachable given the
  wrapper's pre-checks).
- **The swap** needs no Loom target: clone and swap happen on one task; the only shared edge
  is the `spawn_blocking` result channel, already covered by the concurrency.md targets.
