---
title: Formal Verification
last_updated: 2026-08-17
tags:
  - formal-verification
  - lean4
  - proof-scope
  - trust-base
---

# Formal Verification

Verity's headline claim is that the running consensus core is *shown* to match the
specification, not merely believed to. A claim like that is only as strong as its precise
form. This page states it precisely: which artifact is proven, against what specification,
under which assumptions, and where the guarantee stops.

## The specification chain

There is no formal specification upstream. [leanSpec](https://github.com/leanEthereum/leanSpec)
— the working source of truth per the [design philosophy](../design-philosophy.md) — is a
Python reference implementation, and the Lean Ethereum roadmap's formal-verification track
targets cryptographic proof systems (ArkLib), not the consensus protocol. Verity therefore
maintains the formal layer itself. The chain from specification to running artifact, with
what carries fidelity at each link:

| Link | Artifact | Fidelity carried by |
|---|---|---|
| Ground truth | leanSpec (Python, moving) | — |
| Transcription | [formal-leanSpec](https://github.com/NyxFoundation/formal-leanSpec): a Lean 4 model of leanSpec | **Evidenced, not proven** — every Lean file cites the Python source it mirrors; review and leanSpec-derived vectors keep the gap visible |
| Proof | The proposition catalog: theorems about the model | **Machine-checked** by the Lean 4 kernel; `sorry`-free |
| Artifact | Verity Consensus: the compiled, exported subset of the model (Lean C backend → static library → C ABI) | **Trusted** — the C backend and linking sit outside every proof (see [the trust base](#the-trust-base)) |
| Runtime | The Rust shell around the artifact | **Checked, not proven** — the panic-free bar plus the [model-checking strategy](https://github.com/NyxFoundation/verity/blob/main/MODEL_CHECK.md) |

## One model, two roles

formal-leanSpec is a single Lean 4 codebase, but its content ships in two different ways —
and the distinction *is* the verification boundary:

- **Compiled and exported.** The functions that occupy the Verified Core — today the state
  transition and the fork-choice decision — are compiled by Lean's C backend and exported
  over the C ABI as **Verity Consensus**. For these, the theorem and the production code are
  the *same definition*: what is proven is what runs.
- **Proof-only.** The rest of the catalog — validator duties, networking bounds, storage
  atomicity, the sync FSM — is proven about the model, but the production implementation of
  those concerns is Rust, in the Runtime Shell and I/O Edge. There the theorem is a
  **design-basis guarantee**: it fixes what the Rust must uphold. The model-checking
  toolchain (Kani, proptest/bolero, Loom, Miri) is what will carry that obligation onto the
  implementation — it is phased in per the
  [model-checking strategy](https://github.com/NyxFoundation/verity/blob/main/MODEL_CHECK.md),
  not wired in from day one, so until a harness targets a given proposition, that
  proposition constrains the design but says nothing yet about the running Rust.

Which function sits on which side is a snapshot, not a definition — the same
movable-boundary discipline as the [architecture](../reference/architecture.md#boundary-migration):
pulling a proof-only function into the export set, or pushing one out, is a re-binding of a
capability contract, not a redesign.

## The proposition catalog

The unit of proof scope is the **proposition**: a predicate-form statement extracted from
leanSpec, ID-tagged, and proved as a Lean theorem. The catalog lives in formal-leanSpec
(`docs/lean4-proof-propositions.md`) and is the source of truth for what is proven. As of
July 2026 it holds 31 propositions across eight domains — 30 proved, one cryptographic
axiom, zero `sorry`s.

| Domain | IDs | Highlights | Role |
|---|---|---|---|
| SSZ & primitives | SSZ-1..7 | encode/decode round-trips; length and range invariants; `hash_tree_root` collision resistance (**axiom**) | Compiled where exported |
| Containers | CONT-1..2 | checkpoint ordering; justifiability ⇔ δ ≤ 5 ∨ square ∨ pronic | Compiled |
| State transition | ST-1..6 | slot advancement; checkpoint monotonicity; **finalization irreversibility** | Compiled |
| Fork choice | FC-1..5 | head determinism; **head descends from latest justified**; acyclicity; production-loop termination | Compiled (decision functions; the mutable `Store` stays in Rust) |
| Validator | VAL-1..5 | unique proposer; dual-key distinctness; no double-vote; **XMSS window never rewinds** | Proof-only → `verity-validator` |
| Networking | NET-1..2 | req/resp and payload bounds (DoS resistance) | Proof-only → `verity-p2p` |
| Storage | STOR-1..2 | parent presence; batch atomicity | Proof-only → `verity-db` |
| Sync | SYNC-1..2 | FSM closure; gossip gating | Proof-only → orchestrator |

The catalog is also where the **Partnership** with upstream is real, not aspirational:
proving VAL-2 exposed that leanSpec documented but never enforced the proposal/attestation
key distinction — reported upstream and fixed as
[leanSpec#1184](https://github.com/leanEthereum/leanSpec/issues/1184).

## Preconditions are named invariants

The theorems are not unconditional. They are proved relative to explicit well-formedness
predicates — `Store.WellFormed` for the fork-choice store, `AnchorWF` (discharged by
`Reachable` for every state reachable from genesis) for the state,
`ValidatorRegistry.WellFormed` for validator keys. These predicates are the precise content
of the capability contracts' "already-verified inputs": the Runtime Shell owns the mutable
store and manufactures the core's inputs, so **maintaining these predicates across every
mutation is the Rust side's half of the contract** — and the primary target for the Kani
and property harnesses at the boundary. A theorem about a `WellFormed` store says nothing
about a store the shell has corrupted; keeping the shell honest is what the model-checking
strategy is for.

## The trust base

What must be trusted for the claim to hold — listed so no link stays implicit:

- **The Lean 4 kernel** that checks the proofs.
- **The transcription.** formal-leanSpec is a hand transcription of a Python spec; its
  fidelity is evidenced (per-file source tracing, review, shared vectors), not proven.
- **The predicate mirrors.** The well-formedness predicates the theorems assume
  (`Store.WellFormed`, `AnchorWF`, `ValidatorRegistry.WellFormed`) are Lean definitions;
  for the Runtime Shell to maintain them — and for the boundary harnesses to target them —
  they must be hand-transcribed into Rust. That is a **second transcription** of the same
  kind as formal-leanSpec itself, and it is held to the same discipline: each Rust mirror
  cites the Lean definition it mirrors, and shared vectors exercise both sides. A drifted
  mirror silently voids the boundary checks, which is why this link is listed rather than
  assumed.
- **The Lean runtime's abort path.** The compiled artifact embeds the Lean runtime, which
  can abort the process on allocation failure. This is an *availability* residue, classed
  with a Rust-side OOM abort — the panic-freedom claim asserts that no path returns an
  incorrect consensus value, not that a linked runtime can never abort (see the
  [error model](../reference/architecture.md#capability-contracts)). Exported functions are
  total, so no other runtime-panic path exists inside the export set.
- **The artifact pipeline.** Lean's C backend, the generated C, the C compiler and linker,
  and the runtime the artifact embeds. No tool in the project checks that the static
  library computes the proven functions.
- **Cryptography.** `hash_tree_root` collision resistance is an axiom (SSZ-7); the algebra
  of Poseidon / KoalaBear and XMSS is ArkLib's domain and enters the model only at call
  sites; leanVM's implementation is trusted in the Runtime Shell.
- **The Rust toolchain and shell.** rustc, and the Runtime Shell / I/O Edge code — held to
  the panic-free bar and model checking rather than proof.
- **Hardware and OS**, under everything.

What is *not* trusted: the consensus logic itself. Inside the export set, behavior is
either proven or it does not ship.

## Out of scope — and where it lives instead

- **Protocol-level safety and liveness** (e.g. accountable safety of the finality gadget)
  are properties of the *protocol design*, not of a client's conformance to it. They are
  pursued in dedicated research repositories —
  [goldfish-fv](https://github.com/NyxFoundation/goldfish-fv) (the fork choice planned to
  replace 3SF-mini at pq-devnet-5),
  [minimmit-fv](https://github.com/NyxFoundation/minimmit-fv),
  [simplex-fv](https://github.com/NyxFoundation/simplex-fv) — and by upstream researchers
  ([fradamt/verified-consensus](https://github.com/fradamt/verified-consensus);
  [ssf-mc](https://github.com/freespek/ssf-mc)'s bounded model checking of full 3SF).
  Verity consumes the protocol; those efforts justify it.
- **ZK execution proofs.** Proving that one execution of the STF was faithful is
  complementary to proving the STF correct for all inputs.
  [verifiable-stf](https://github.com/NyxFoundation/verifiable-stf) prototypes that
  direction for the Lean-written STF (zkVM verification of Lean IR execution traces); the
  full tension is recorded in the
  [Ethlambda notes](https://github.com/NyxFoundation/verity/blob/main/memo.md#open-question-unresolved-zk-proving-the-stf-vs-lean4-verification).
- **Cryptographic primitive algebra**: ArkLib.

## Tracking a moving specification

leanSpec moves — breaking changes between devnets are expected, and pq-devnet-5 plans to
replace 3SF-mini with committee voting under the Goldfish fork choice. The proof scope is
therefore **fork-versioned**: the model and catalog are pinned to the lstar fork and
re-grounded as upstream advances, with the sync discipline visible in formal-leanSpec's
history (upstream fixes #1177–#1184 flowed both ways). Minimalism keeps the re-proof cost
of a fork change bounded; the movable boundary keeps the change a re-binding.
