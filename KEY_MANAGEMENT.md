# Validator Key Management — XMSS Signing State

> Status: pre-implementation. Decisions ratified 2026-08-16. This document settles how Verity
> manages its validators' XMSS keys: the crash-safe no-reuse guarantee, key material loading,
> and preparation scheduling. It extends [STORAGE.md](STORAGE.md) with one column family and
> plugs into the runtime model of [CONCURRENCY.md](CONCURRENCY.md).

The stake is unusual and worth stating first. XMSS is a **stateful one-time signature
scheme**: signing two *different* messages with the same key at the same epoch does not incur
a protocol penalty — the lean protocol has no slashing at all — it **breaks the key
cryptographically**, exposing enough one-time-chain state that signatures can be forged. The
asset being protected is the secret key's soundness itself. Everything below is sized against
that, not against a fine.

## What the spec and the library fix

Read at leanSpec `main` = `cce7955`, leanSig `main` = `c08a3ba`.

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
  one slot; no slashing container exists. The reference node keeps signing state in memory
  only and persists nothing across restarts.

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

## Survey — what other clients do

Read at ethlambda `e16f477`, ream `842a709`, zeam `6495beb`, qlean-mini `55b6eb3`.

| Client | Persisted signing state | Anti-double-sign guard | `advance_preparation` |
|---|---|---|---|
| ethlambda | none | none | per-slot pre-advance on the chain actor + startup catch-up |
| ream | none | in-memory block-prebuild flag only | **never called in production code** |
| zeam | none | none; **falls back to one key for both roles** when only one file is present | lazily, inline inside `sign`, under the signing mutex |
| qlean-mini | none | none | none |

Every client derives the signing epoch from the wall clock at the moment of signing and
persists nothing. The entire ecosystem's no-reuse guarantee is an implicit bet on clock
monotonicity. The survey also produced two live counterexamples of what unmanaged key state
looks like: ream will panic at the prepared-window boundary because nothing ever advances it,
and zeam's single-key fallback reproduces exactly the dual-role reuse the spec's reference
loader rejects.

Three scenarios break the clock-monotonicity bet:

1. **Intra-slot crash-restart.** The process restarts within one 4-second slot; the node's
   view (head) has changed; the validator re-attests the same slot with different data — two
   different messages under one `(key, slot)`. A restarted proposer rebuilding its block hits
   the same trap.
2. **Clock step-back** (NTP correction, VM resume) — the derived slot rewinds into
   already-signed territory.
3. **Operational error** — restoring a datadir from backup, or accidentally running two
   instances with the same keys.

## Decision 1 — signing watermark, persist-before-sign

Verity persists a **signing watermark**: the last slot signed, per `(validator, role)`, in a
dedicated `verity-db` column family (`signing_watermarks`, defined in
[STORAGE.md](STORAGE.md#column-families)). The signing path enforces, in this order:

1. derive the message for slot `s`;
2. require `s > watermark(validator, role)` — **equality is refused**, even for a
   byte-identical message;
3. durably write `watermark := s` (write-ahead log + fsync);
4. only then sign and release the signature to the rest of the process.

A signature therefore cannot exist — not in channel ②, not on the wire — unless a watermark
at or above its slot is already on disk. A crash between steps 3 and 4 costs exactly one
duty (one vote or one proposal in the crash slot), which carries no protocol penalty; the
alternative it buys off is key compromise. Refusing equality (rather than allowing the
idempotent re-sign leanSig's determinism would permit) is deliberate: the rescue value of one
duty in a 4-second-slot protocol is negligible, and slot-only state keeps the mechanism two
`u64`s per validator instead of a message-root log.

- **Ownership.** The validator signing path is the sole writer of `signing_watermarks` — the
  one documented exception to the chain task's write ownership (see STORAGE.md). This keeps
  the check-write-sign sequence synchronous inside the signing path; routing it through the
  chain task would add a query channel that [CONCURRENCY.md](CONCURRENCY.md) deliberately
  does not have. One keyspace, one writer still holds — per family, not per database.
- **Write cost.** At most two fsynced single-row writes per slot per validator; negligible
  against the storage engine's proof workload.
- **Startup.** Watermarks are read before validator readiness. If a watermark is *ahead* of
  the current wall-clock slot, the clock has rewound (scenario 2 or 3): the node fails closed
  — duties for that validator stay disabled until the clock passes the watermark, and the
  condition is logged loudly. It is never treated as corruption to repair automatically.
- **Scenario coverage.** (1) is covered by the equality refusal, (2) by the startup check plus
  the per-sign comparison, (3) partially: a restored backup restores an old watermark, but the
  per-sign comparison still refuses any slot at or below it *if the restored node's clock is
  honest*; two concurrent instances sharing keys are out of scope of any local mechanism and
  remain an operator responsibility, stated in the operator documentation.

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
  the worker receives a **clone** (a ~33.5 MB memcpy), advances the clone off-thread while
  the original keeps signing in the still-valid old window, and the validator task swaps the
  advanced clone in between sign calls. The task is single-threaded, so the swap is a plain
  field replacement with no torn state. Both key objects are the same key; no-reuse is
  enforced by the watermark on the signing path, independent of which clone signs.
- **Persist the advanced key.** After each successful advance, the key file is rewritten
  atomically (temp file + rename; public key verified against the manifest on load). No
  surveyed client does this, and the omission is why a year-old node would owe ~120
  consecutive rebuilds (minutes to hours) at startup: the on-disk key never moves off its
  activation window. With periodic persistence, startup catch-up is bounded by the downtime,
  not the node's age. Safety is unaffected — a stale key file only costs extra catch-up, and
  the watermark, not the key file, carries the no-reuse guarantee.
- **Panic containment.** leanSig's `sign` asserts; Runtime Shell code must not panic. The
  `verity-crypto` wrapper checks the activation and prepared intervals first and returns a
  typed error — the assert becomes unreachable, and Kani's target is exactly that
  unreachability.
- **Startup order.** Load keys → read watermarks (clock-rewind check) → advance each key
  until the current slot is inside its prepared interval (on `spawn_blocking`; may take a
  while after long downtime, progress logged) → only then validator readiness. This extends
  the [CONCURRENCY.md](CONCURRENCY.md#lifecycle) startup sequence: validator-duty tasks wait
  on the first `ChainView` *and* on their keys being prepared.

## Deliberately out of scope

- **Remote signers** and any signing API surface.
- **Key generation ceremony / tooling** — devnet keys come from lean-quickstart's generator;
  Verity ships no keygen.
- **Hot key reload** — key set changes require a restart.
- **A separate slashing-protection database** — the protocol has no slashing; the watermark
  *is* the double-sign protection, and an eth2-style interchange format has no counterpart
  here yet.

## Verification obligations introduced by this model

- **The watermark ordering property** is the consensus-critical invariant of this document:
  *no signature is released whose slot is not durably watermarked*. It is a
  sequential-ordering property of the signing path, targeted with property tests
  (crash-injection between write and release) rather than Loom — no concurrency is involved.
- **Kani / bolero:** the loader (no-panic-on-any-input over manifest and key bytes; reject ⇒
  no partial registry) and the sign wrapper (leanSig's asserts unreachable given the
  wrapper's pre-checks).
- **The swap** needs no Loom target: clone and swap happen on one task; the only shared edge
  is the `spawn_blocking` result channel, already covered by the CONCURRENCY.md targets.
