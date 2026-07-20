# Ethlambda Comparison Notes

This memo focuses on the four ethlambda crates that matter most for Verity's
Lean/Rust verification boundary:

- `ethlambda-lean-ffi` -> `verity-consensus-sys`
- `ethlambda-state-transition` -> Verity Consensus / `verity-chain` boundary
- `ethlambda-fork-choice` -> Verity Consensus / `verity-chain` boundary
- `ethlambda-blockchain` -> `verity-chain` + `verity-validator` + `verity` binary

## High-Level Difference

Ethlambda is organized around a Rust consensus implementation, with selected
Lean functions inserted through FFI behind a Cargo feature. Verity should invert
that emphasis: Verity Consensus is the verified Lean implementation, and Rust is
the trusted shell that prepares inputs, calls it, owns mutable runtime state, and
persists results.

In short:

```text
ethlambda:
  Rust blockchain implementation first
  Lean is currently a local replacement for selected functions

Verity:
  Verity Consensus first
  Rust coordinates verified calls and owns effects outside the proof boundary
```

## 1. FFI Boundary

Ethlambda's `ethlambda-lean-ffi` is a narrow FFI crate. In the
`lean-formalization` branch, the observed switch point is
`slot_is_justifiable_after`: with the `lean-ffi` feature enabled, Rust calls the
Lean implementation; without it, Rust uses the native implementation.

Verity's `verity-consensus-sys` should be broader and more explicit. It should
be the only raw ABI boundary for Verity Consensus. It is the **swappable backend
behind the capability contracts**: its export set is exactly whatever Verified Core
currently hosts, and is expected to expand or contract as the verification
boundary moves (see [boundary migration](ARCHITECTURE.md#boundary-migration)).

Responsibilities:

- declare `extern "C"` functions exported by Verity Consensus
- initialize and manage the Lean runtime
- isolate `unsafe`
- handle ABI-level ownership and representation
- expose the smallest practical low-level Rust surface for `verity-chain`

Non-responsibilities:

- no chain orchestration
- no database access
- no P2P, RPC, metrics, or validator duties
- no consensus policy beyond raw calls into Verity Consensus

> Open: whether Verity Consensus (and therefore this FFI boundary) should host the
> STF at all is unsettled — see *Open question (unresolved): ZK-proving the STF vs
> Lean4 verification* below.

## 2. State Transition Boundary

Ethlambda's `ethlambda-state-transition` owns the Rust implementation of the
state transition. Its `state_transition(&mut State, &Block)` path is Rust-first:
it mutates `State`, calls Rust `process_slots`, and calls Rust `process_block`.
Lean currently replaces only selected helper logic.

Verity should place the state transition itself in Verity Consensus.

Target split:

```text
Verity Consensus:
  state_transition(pre_state, verified_block) -> post_state

verity-chain:
  find parent state
  accept already-decoded and already-verified block input
  call Verity Consensus
  handle result/error
  persist block and post-state through verity-db
  coordinate finalized/head updates with Store
```

`verity-chain` should not reimplement the state transition. If a Rust fallback
exists, it should implement the same Verity Consensus boundary for testing,
conformance, or development, not grow as a parallel ad hoc path. That "same
boundary" is precisely the `StateTransition` capability contract: a native-Rust
implementation of it *is* the Runtime-Shell placement of the STF, so a development
fallback and an eventual Verified Core → Runtime Shell migration are the same mechanism, not two (see
[boundary migration](ARCHITECTURE.md#boundary-migration)).

> Open: this section assumes the STF lives in Lean. The Lean Ethereum roadmap
> points toward ZK-proving the consensus STF, which pulls the STF toward a
> zkVM-friendly language — see *Open question (unresolved): ZK-proving the STF vs
> Lean4 verification* below.

## 3. Fork Choice Boundary

Ethlambda's `ethlambda-fork-choice` is close to the shape Verity wants. Its
`compute_lmd_ghost_head(start_root, blocks, attestations, min_score)` style is
mostly pure: it takes a view of blocks and attestations and returns a head plus
weights.

Verity should keep that purity but move the consensus-critical decision
function into Verity Consensus.

Target split:

```text
verity-chain:
  owns the mutable Store as a single writer
  extracts StoreView / block graph / latest attestations
  applies returned head, safe_target, and checkpoint updates

Verity Consensus:
  fork_choice_decision(view) -> head / safe_target / updated view
```

The important distinction is ownership versus decision:

- Rust owns the mutable Store and persistence.
- Lean decides the consensus-critical transition from immutable inputs.

This keeps the proof surface pure while avoiding a large mutable database-backed
Store inside Lean. Fork choice is therefore the worked example of a capability
*split* across the boundary — a decision function in Verified Core over a `Store` owned
in Runtime Shell (see [boundary migration](ARCHITECTURE.md#boundary-migration)).

## 4. Chain Orchestration Boundary

Ethlambda's `ethlambda-blockchain` is a practical integration crate, but it is
too broad for Verity's verification-boundary-first architecture. It combines:

- Store ownership
- pending block handling
- tick handling
- inbound block import
- inbound attestation import
- block proposal
- attestation production
- signing
- P2P publishing
- key management

Verity should split these responsibilities.

```text
verity-chain:
  owns State and Store
  is the single writer for consensus state
  processes inbound consensus events
  handles pending blocks
  sequences Verity Consensus calls
  coordinates persistence through verity-db
  exposes read APIs for head, finalized checkpoint, and state views

verity-validator:
  owns local validator duties
  requests chain views or candidate data from verity-chain
  signs blocks and attestations
  coordinates aggregation and outbound validator artifacts
  does not own Store or decide fork choice

verity binary:
  wires runtime components
  owns slot clock scheduling and bounded queues
  starts P2P, RPC, metrics, chain, and validator services
```

The key rule is that `verity-chain` is the only writer of consensus state, while
`verity-validator` produces local validator actions from chain views. This keeps
validator production from leaking into consensus state ownership.

## Design Implication

Ethlambda is valuable as a reference for incremental Lean adoption and a working
Rust client structure. Verity should borrow the useful crate boundaries, but not
copy the central `ethlambda-blockchain` aggregation.

The Verity boundary should be defined around these APIs early:

```text
verity-chain -> verity-consensus-sys -> Verity Consensus
```

This gives Verity a clear verified implementation boundary while still allowing
Rust-side fallbacks or test implementations to target the same interface.

## Open question (unresolved): ZK-proving the STF vs Lean4 verification

Sections 1 and 2 assume the consensus state transition lives in Verity Consensus
(Lean 4) behind `verity-consensus-sys`. That assumption is recorded here as **open,
not settled**. No decision is changed in this memo — this section only captures the
tension and the trigger for revisiting it.

The architecture is built to *withstand* this move regardless of how it resolves:
relocating the STF is a re-binding of the `StateTransition` capability contract from
an FFI-into-Lean implementation to a native / zkVM one, not a redesign. This is the
worked Verified Core → Runtime Shell example in [boundary migration](ARCHITECTURE.md#boundary-migration).

### What surfaced it

- ethlambda closed its Lean4-STF formalization PR without merging
  (`lambdaclass/ethlambda#269`, formalizing `slot_is_justifiable_after`). The stated
  reason: *"ZK proving the STF is in the roadmap, and moving to Lean4 would get in
  the way of that."* That PR targeted a 3SF consensus function — the same surface
  Verity places inside Verity Consensus.
- The Lean Ethereum roadmap points toward SNARK-proving consensus components. As of
  2026-07 the public tracker's zkVM track covers PQ signature aggregation
  (pq-devnet-4/5 block-level aggregation proofs); SNARK-proving the STF itself has no
  published spec or roadmap phase — the direction is attested by statements like the
  one above, not by a roadmap item.

### Why it is not a conflict today

- Current lstar proofs (`SignedBlock.proof`, Type-1/Type-2) prove **signature
  aggregation**, not STF execution. Verity verifying those aggregate proofs in
  `verity-crypto` is consistent with the current design. STF-proving is the
  longer-term L* evolution and has **no specification yet**.

### The actual tension (forward-looking)

- Formal verification (Lean 4) and ZK execution proofs answer **different
  questions**. Lean proves the STF *implementation* is correct for all inputs
  (static, universal). A ZK proof proves *one* execution was faithful to the program
  that ran — it does **not** prove that program is correct; a ZK proof of a buggy STF
  faithfully proves the bug. So ZK-proving does not subsume Verity's "the running
  client is shown to match the spec" thesis; the two are complementary (cf. the
  Ethereum Foundation's separate zkEVM formal-verification effort, which exists
  precisely because ZK circuits still need their correctness proven).
- The binding constraint is **artifact/language**: a single artifact cannot be both
  Lean4-proven and efficiently zkVM-proven. Lean's runtime (reference counting,
  boxed values, GC-style allocation) is hostile to in-zkVM execution; the zkVM path
  is Rust→RISC-V or a leanVM zkDSL. This is why ethlambda kept the STF in Rust.

### Working position and revisit trigger

- **Working position (unchanged):** the STF stays in Verity Consensus (Lean 4), as
  in Sections 1 and 2. This is *undecided, not reversed* — Verity's differentiator
  is formal verification, and the conflicting roadmap item does not exist as a spec
  yet.
- **Revisit trigger:** when an L* "real-time CL proofs" specification for the
  consensus STF materializes upstream, reconsider where the STF lives and what
  `verity-consensus-sys` is for. Candidate reconciliations to evaluate then:
  - Lean 4 as the verified source of truth, with a separate, equivalence-checked
    zkVM artifact for proof generation; or
  - a zkVM-native STF (Rust / leanVM zkDSL) with Lean 4 proving properties only
    (the ethlambda shape), accepting the loss of "verified running client matches
    spec".

  An active prototype of the first reconciliation exists:
  [NyxFoundation/verifiable-stf](https://github.com/NyxFoundation/verifiable-stf)
  interprets the Lean 4 IR of the Lean-written STF on the host and verifies each trace
  step in a RISC Zero guest — keeping Lean as the verified source of truth while a zkVM
  proves *executions* of it. If that scales, the artifact/language constraint above
  dissolves rather than forcing a side.
