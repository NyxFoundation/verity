# Model-Checking Adoption Strategy

> Internal planning memo — not part of the published mdBook. Strategy level only:
> it decides *which* model-checking techniques apply *where*, and *why*. It does not
> prescribe CI configuration, `cargo` invocations, or crate layout. Verity is
> pre-implementation; this memo is meant to settle the strategy before the first Rust
> crate exists, so the architecture is built to be model-checkable from day one.

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
  *given* clean, typed, in-range inputs; the Runtime Shell and Boundary that manufacture
  those inputs are unproven Rust. Whether that Rust is actually panic-free and
  range-correct is a separate, checkable claim — independent of the proof, not a re-check
  of it.
- **The boundary code**, where promotion/lowering and range checks live — bounded,
  input-driven logic that a bounded checker fits naturally and cheaply.

A note on the ecosystem: neither `ream` nor `ethlambda`, the reference Lean Consensus
Rust clients, use model checkers today. Both rely on leanSpec test vectors plus live
multi-client devnet interop. Adopting model checking is therefore novel here, and it is
a direct expression of Verity's **"proof over test"** stance — the floor those clients
stand on is the floor Verity builds above.

## The zones, and what checks each

The architecture is organized around a **moving verification boundary** (see
`docs/src/reference/data-representation.md`): the proven core, the boundary, and the
edges. Per `ARCHITECTURE.md` the edges split by guarantee into the panic-free **Runtime
Shell** and the concurrent **I/O Edge** — a distinction that matters here, because
concurrency checking applies only to the latter. Each zone is owned by a different primary
technique; model checking plays a specific, bounded role in each.

| Zone | Primary owner | Model-checking role |
|---|---|---|
| **Proven Core** (state transition, fork choice) | Lean 4 (deductive proof) | None inside the core — it is Lean, owned by the proof. Kani instead checks the Rust FFI wrapper that feeds it (see Boundary) |
| **Boundary** (promote ↔ lower, range & well-formedness checks, SSZ) | — (no single owner yet) | Bounded model checking + property testing: round-trip, no-panic-on-any-input, range enforcement |
| **Runtime Shell** (storage, DB, SSZ, crypto, FFI bindings — Rust, panic-free) | Ordinary high-quality Rust | Dynamic UB detection (Miri) on `unsafe` / FFI / crypto bindings |
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
  Its natural home is the **Boundary**: promote/lower conversions and SSZ are exactly the
  bounded, input-driven code where a no-panic-on-any-byte-sequence and a
  `lower ∘ promote = id` round-trip property are cheap to state and decide.

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
  Runtime Shell / Boundary alongside the Lean proof of the core. Verity deliberately keeps
  a single deductive stack — **Lean 4** — and does not add a second one unless that
  strategy changes; model checking complements the Lean path rather than duplicating
  deductive effort.

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
