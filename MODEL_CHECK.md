# Verification Tooling Adoption Strategy

> Internal planning memo — not part of the published mdBook. Strategy level only:
> it decides *which* verification techniques apply *where*, and *why*. It does not
> prescribe CI configuration, `cargo` invocations, or crate layout. Verity is
> pre-implementation; this memo is meant to settle the strategy before the first Rust
> crate exists, so the architecture is built to be model-checkable from day one.
>
> "Model checking" is the headline, but not everything below is a model checker. The
> adopted tools span a spectrum — deductive proof, bounded model checking, exhaustive
> and randomized concurrency exploration, property testing/fuzzing, and dynamic
> UB detection. Calling them all "model checking" would overstate the weaker ones, so
> this memo classifies each by **assurance strength** (how it explores state) and by
> **zone** (where it applies). Those are the two axes the whole strategy hangs on.

## The core claim

Verity's verification path is a **Lean 4 deductive proof of the Verified Core**: the
consensus logic is written as pure, total Lean functions and proven correct, then
compiled via Lean's C backend into a static library and consumed by the Rust runtime over
a C ABI (see `ARCHITECTURE.md`). That deductive proof is the ceiling for the *functional
correctness* of the proven core, and nothing here displaces it. Model checking is adopted
as a **complement**, never a replacement — it earns its place in exactly three situations
the deductive proof leaves open:

- **Zones the deductive proof structurally cannot reach.** The proof reasons only about
  the pure, total, *sequential* Lean core. It says nothing about `unsafe`, interior
  mutability, **concurrency**, trait objects, I/O, or the FFI seam. Everything Verity must
  run that lives outside that core — the whole Rust runtime that wraps it — is, by
  construction, outside the Lean guarantee, and that is precisely where a different
  technique has to carry the assurance.
- **The Rust-side preconditions the proof depends on.** The proof discharges correctness
  *given* clean, typed, in-range inputs; the Runtime Shell code that manufactures
  those inputs is unproven Rust. Whether that Rust is actually panic-free and
  range-correct is a separate, checkable claim — independent of the proof, not a re-check
  of it.
- **The boundary code in the Runtime Shell**, where promotion/lowering and range checks
  live — bounded, input-driven logic that a bounded checker fits naturally and cheaply.

A note on the ecosystem: neither `ream` nor `ethlambda`, the reference Lean Consensus
Rust clients, use model checkers today. Both rely on leanSpec test vectors plus live
multi-client devnet interop. Adopting model checking is therefore novel here, and it is
a direct expression of Verity's **"proof over test"** stance — the floor those clients
stand on is the floor Verity builds above.

## Two axes: assurance strength × zone

The tools are not one kind of thing. Before mapping them, separate them by **what they
actually do to the state space** — this is the vertical axis, ordered strongest to
weakest. "Model checker" applies to only three rows; the rest are testing or dynamic
analysis, and saying so is the point.

| # | Technique class | Is it model checking? | Guarantee it gives | Tool |
|---|---|---|---|---|
| 1 | **Deductive proof** (unbounded) | No — it's proof | Correct for *all* inputs, forever | Lean 4 |
| 2 | **Bounded model checking** | **Yes** | Exhaustive up to a size bound; sound below it | Kani |
| 3 | **Exhaustive concurrency MC** | **Yes** | All interleavings in a small space | Loom |
| 4 | **Randomized concurrency exploration** | Model-checker-shaped, unsound | Samples interleavings; a pass is evidence, not proof | Shuttle |
| 5 | **Property testing / fuzzing** | No — it's testing | Random/guided inputs; finds counterexamples, proves nothing | proptest, bolero |
| 6 | **Dynamic UB detection** | No — it's a runtime sanitizer | UB only on paths actually executed | Miri |

Strength drops monotonically down the table: 1 proves, 2–3 exhaust a bounded space, 4
samples a space, 5 samples inputs, 6 watches one run. Each zone gets the strongest row
its nature allows — a pure sequential core can reach row 1; a concurrent I/O subsystem
tops out at rows 3–4.

The horizontal axis is the **zone** (per `ARCHITECTURE.md`): Verified Core, the
boundary code that services the moving seam (physically in the Runtime Shell), the rest
of the Runtime Shell, and the I/O Edge. Crossing the two axes gives the whole plan on
one grid — a cell is filled only where that technique is both *applicable* and *the
strongest available* for that zone:

| Assurance strength ↓ / Zone → | Verified Core (pure, sequential) | Boundary code (promote/lower, SSZ, FFI wrapper) | Runtime Shell (`unsafe`/FFI/crypto) | I/O Edge (concurrent) |
|---|---|---|---|---|
| **1. Deductive proof** — Lean 4 | ✅ **Lean 4** (owner) | ⬆ target once promoted | — | — |
| **2. Bounded MC** — Kani | — | ✅ **Kani** (panic-free, overflow, `lower∘promote=id`) | — | — |
| **3. Exhaustive concurrency MC** — Loom | — | — | — | ✅ **Loom** (small interleaving spaces) |
| **4. Randomized exploration** — Shuttle | — | — | — | ✅ **Shuttle** (spaces too large for Loom) |
| **5. Property test / fuzz** — proptest + bolero | — | ✅ unbounded counterpart to Kani | — | ✅ VAL-* invariants, req/resp bounds |
| **6. Dynamic UB** — Miri | — | — | ✅ **Miri** (aliasing, races, OOB) | — |

Reading the grid: the Verified Core is the only zone that reaches row 1, and nothing
weaker is applied inside it. The boundary code is the busiest column — Kani (row 2) and
bolero (row 5) both apply, because bounded exhaustiveness and unbounded sampling are
complementary there. The I/O Edge cannot go above row 3 by construction (it is
concurrent and impure), which is exactly why purity was pushed out of the core. Miri
(row 6) is the *only* technique for the Runtime Shell's `unsafe`/FFI/crypto, so a weak
guarantee there is a deliberate, acknowledged floor — see Honest limits.

## The zones, and what checks each

The architecture has exactly **three zones**, defined by guarantee level in
`ARCHITECTURE.md`: Verified Core, Runtime Shell, and I/O Edge. The **verification
boundary** is the moving seam between Verified Core and Runtime Shell — a line, not a
zone of its own (see `docs/src/reference/data-representation.md`). The Rust that
services that seam — promote ↔ lower conversions, range and well-formedness checks,
SSZ, the FFI wrapper — is **boundary code**, and it lives in the Runtime Shell. Each
zone is owned by a different primary technique; model checking plays a specific,
bounded role in each.

| Zone | Primary owner | Model-checking role |
|---|---|---|
| **Verified Core** (state transition, fork choice — Lean 4, pure) | Lean 4 (deductive proof) | None inside the core — it is Lean, owned by the proof. Kani instead checks the Runtime Shell boundary code that feeds it |
| **Runtime Shell** (boundary code, storage, DB, SSZ, crypto, FFI bindings — Rust, panic-free) | Ordinary high-quality Rust | On boundary code: bounded model checking (Kani) + property testing — round-trip, no-panic-on-any-input, range enforcement. On `unsafe` / FFI / crypto bindings: dynamic UB detection (Miri) |
| **I/O Edge** (networking, slot clock, validator duties, RPC, metrics, orchestration — Rust, concurrent) | Ordinary high-quality Rust | Concurrency model checking (Loom / Shuttle) — the only concurrent zone, which the deductive proof does not reach |

## Tool-to-zone mapping

Each tool is adopted for a specific zone and tied to a stated belief from the
[design philosophy](../docs/src/design-philosophy.md). The mapping — not the tool list —
is the decision.

- **Kani** (AWS; bounded model checker, CBMC backend). Verifies absence of panics,
  arithmetic overflow/underflow, and memory-safety faults on the *real* Rust (via MIR),
  for all inputs up to a bound. This is the headline adoption. It serves two beliefs at
  once: **"Panic-freedom as a proof contract"** and **"Arithmetic that mirrors proven
  invariants"**. Its domain is the **Rust-side boundary and FFI-wrapper code** — the
  unproven Rust that surrounds the Lean core — not the Lean core itself, which Kani cannot
  reach (and it does *not* validate the Lean C backend or the static library it emits).
  Its natural home is the **boundary code in the Runtime Shell**: promote/lower
  conversions and SSZ are exactly the bounded, input-driven code where a
  no-panic-on-any-byte-sequence and a `lower ∘ promote = id` round-trip property are
  cheap to state and decide.

- **proptest + bolero** (property testing / fuzzing front-ends). Provide the *unbounded*
  counterpart to Kani's bounded checks. The reason bolero specifically matters: a single
  harness can be driven as a property test, as a coverage-guided fuzzer, *and* as a Kani
  proof. One harness, three engines, the same leanSpec-derived vectors flowing through
  all of them — this is **"Conformance through shared evidence"** made concrete: the
  Rust implementation and the formal effort are exercised against identical inputs and
  the gap between them stays visible.

- **Loom** (exhaustive, small interleaving spaces) and **Shuttle** (randomized,
  scales to large concurrent subsystems). Concurrency model checkers for the **I/O Edge**,
  the only zone where concurrency and the outside world live. This is a zone the deductive
  proof cannot reach, and it is non-empty *by design*: the belief **"Pure, deterministic
  Verity Consensus"** deliberately pushes concurrency and I/O to the edge so the core stays
  provable. That same decision concentrates all the interleaving, ordering, and data-race
  risk into the I/O Edge — so it is exactly where interleaving exploration earns its keep.
  Use Loom where the interleaving space is small enough to explore exhaustively; reach for
  Shuttle when it is not.

- **Miri** (dynamic undefined-behavior detector). For the `unsafe`, FFI, and
  cryptographic-binding code in the Runtime Shell — code that necessarily falls outside
  the proven pure core. Catches aliasing violations (Stacked/Tree Borrows), data races,
  and unaligned/out-of-bounds access on exercised paths.

- **Not adopted: Creusot, Verus, Prusti.** These are deductive verifiers for *Rust*.
  Adopting one would mean standing up a *second*, Rust-side deductive stack for the
  Runtime Shell alongside the Lean proof of the core. Verity deliberately keeps
  a single deductive stack — **Lean 4** — and does not add a second one unless that
  strategy changes; model checking complements the Lean path rather than duplicating
  deductive effort.

## The proposition catalog supplies the properties

The properties the tools above check are not invented ad hoc. The
[formal-leanSpec](https://github.com/NyxFoundation/formal-leanSpec) proposition catalog
(`docs/lean4-proof-propositions.md`) proves spec-level propositions about the Lean model
across eight domains, and only part of that model is compiled into the shipped artifact
(see `docs/src/concepts/formal-verification.md`). The rest — the **proof-only** domains,
whose production implementations are Rust — hand each proposition to this strategy as a
named implementation obligation:

| Catalog domain | Proposition examples | Rust owner (zone) | Obligation carried by |
|---|---|---|---|
| VAL-1..5 | unique proposer; no double-vote; XMSS window never rewinds | `verity-validator` (I/O Edge) | property tests (bolero); VAL-5 is slashing-critical |
| NET-1..2 | req/resp and payload bounds | `verity-p2p` (I/O Edge) | Kani + bolero on the bound checks |
| STOR-1..2 | parent presence; batch atomicity | `verity-db` (Runtime Shell) | property tests + Miri; atomicity against the embedded KV's transactions |
| SYNC-1..2 | FSM closure; gossip gating | `verity` bin orchestrator (I/O Edge) | Loom / Shuttle on the concurrent FSM |

The boundary invariants work the same way in the other direction: the core's theorems are
proved relative to named predicates (`Store.WellFormed`, `AnchorWF`/`Reachable`,
`ValidatorRegistry.WellFormed`), so the Runtime Shell code that mutates the store owes
their preservation — those predicates are the primary Kani harness targets at the boundary.
This is "Conformance through shared evidence" in a second, stronger form: the same
proposition, proved in Lean about the model and checked in Rust about the implementation.

## Graduated assurance along the moving boundary

The verification boundary is designed to **move**: a component may be pulled into the
proven core once it earns a Lean proof, and pushed back out if proving demands a
different target. Model checking is what makes those crossings a *re-binding rather than
a redesign*, by supplying an interim guarantee at each stage:

> **none → bounded model check (Kani) → deductive proof (Lean)**

A boundary-adjacent component starts with Kani's bounded, automated guarantee while it
still lives at the edge. When it is promoted into the core, its Kani harness is the
specification the Lean proof discharges in full — the bounded check is the scaffolding
the unbounded proof is built on, not throwaway work. Assurance ratchets up monotonically
as a component moves inward; it never has to be re-established from zero.

## Honest limits

Stating what these tools do *not* give is as important as stating what they do.

- **Kani is bounded.** It is a model checker, not an unbounded proof. It must not be
  used to "prove" the state transition correct for arbitrary validator sets or slot
  ranges — that is Lean's job, and conflating the two would overstate the guarantee.
  Kani's value is strong bug-finding plus a sound guarantee *up to the chosen bound*.
- **Loom is not fully C11-sound.** It treats `SeqCst` accesses as `AcqRel`, which can
  produce false alarms; it explores interleavings within bounds, not the full memory
  model.
- **Shuttle is probabilistic.** Randomized scheduling scales but is unsound: a passing
  run is evidence, not a proof of race-freedom.
- **Miri is dynamic.** It only finds undefined behavior on paths that tests actually
  exercise; unexercised `unsafe` is unchecked.
- **The Lean-to-artifact pipeline is unchecked by any tool here.** The Lean proof covers
  the Lean functional model; the Lean C backend, the generated C, the static library,
  linking, and the artifact's functional equivalence to the proof are outside every tool
  in this memo. The C ABI seam between the Rust runtime and the proven core is trusted,
  not verified.
- **Toolchain reality.** Kani ships its own pinned toolchain and its official CI action
  targets Linux/x86_64; Miri requires a nightly toolchain. These constrain *where* the
  checks run, not *whether* they are worth running.

## Summary

| Belief (design philosophy) | Technique that carries it |
|---|---|
| Proof over test | Lean 4 (ceiling); model checking as the complement above the test floor |
| Panic-freedom as a proof contract | Kani (panic-freedom on the Rust-side boundary/FFI wrapper) |
| Arithmetic that mirrors proven invariants | Kani (overflow/underflow on the Rust-side boundary/FFI wrapper) |
| Pure, deterministic Verity Consensus | Loom / Shuttle guard the I/O Edge that purity creates |
| Conformance through shared evidence | proptest + bolero, one harness / many engines, leanSpec vectors |
| The verification boundary is first-class and moves | Graduated assurance: none → Kani → Lean |
| Trust chain from spec to artifact | Kani on the Rust-side boundary/FFI wrapper; Miri on `unsafe`/FFI (the Lean-to-artifact pipeline stays trusted, not verified) |
