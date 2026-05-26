# Design Philosophy

Verity is the Provable Consensus Client. Where other clients test for correctness, Verity proves it. The point of this document is not to list features or pick libraries — it is to state the beliefs that decide every later question. When two reasonable designs compete, the one that keeps Verity *provably* faithful to the specification wins.

Everything below assumes the ordinary disciplines of good software — small functions, single responsibility, clear naming, simplicity over cleverness. Those are the floor, not the philosophy. What follows is what makes Verity different from a merely well-engineered client.

## Why a provable client

Lean Ethereum is a clean-slate redesign of the consensus layer whose explicit goal is to be small enough to reason about completely. The specification is deliberately minimal so that it can be formally verified, not just reviewed. That is a remarkable foundation — but a verifiable specification does not make a correct client. Between the specification and a running binary sits an implementation, and that is where real consensus bugs have always lived: a misread field, an off-by-one in a transition, an aggregation edge case, a panic on malformed input.

Verity exists to close that gap and keep it closed. Its purpose is to carry the guarantees of a verifiable specification all the way into the software that validators actually run, so that the implementation is not merely believed to match the specification but is shown to. This is the single idea from which the rest of the philosophy follows.

## Foundational beliefs

These are the convictions that sit above the design principles. They rarely change; the principles are how we honor them.

- **leanSpec is the working source of truth — and it moves.** leanSpec, the Python reference implementation, is the most authoritative executable definition of Lean Consensus available, and Verity treats it as the practical ground truth for protocol behavior, container shapes, and constants. Where it fixes a shape, Verity matches it exactly: field order and serialization are consensus-critical, because they determine the values that get hashed and signed, so a "harmless" reordering is a consensus fault. But leanSpec is a *reference implementation*, not a frozen or complete authority. Lean Consensus is still evolving in both its design and its implementation, and leanSpec does not always reflect the latest specification discussions — it can trail the live design debate. Verity therefore tracks leanSpec closely while holding it loosely: it conforms to the current target for interoperability and treats an interop-breaking divergence as a Verity defect rather than a local improvement, yet it follows the upstream design discussions — not only the code — to anticipate where the reference is heading, and it expects breaking change between devnets (backwards compatibility is explicitly not a goal) rather than freezing a convenient snapshot.

- **Proof over test.** Conformance vectors and interoperability runs are the floor. A passing test suite shows that the cases we thought of work; it says nothing about the cases we did not. Mathematical proof is the ceiling, and it is the standard Verity holds itself to. "Tested correct" and "proven correct" are different claims, and Verity makes the second one wherever it matters.

- **Minimalism in service of verifiability.** Keeping things small is not an aesthetic preference here; it is a precondition for proof. Every abstraction, generic, and indirection that Verity Consensus must account for widens the surface a proof has to cover and loosens the correspondence between the Rust implementation and its Lean 4 model. So Verity keeps the proven surface as small as the protocol allows, and adds structure only when a real protocol requirement demands it — never in anticipation of one.

- **The verification boundary is a first-class part of the architecture.** Verity is explicit, at all times, about what lives inside Verity Consensus and what lives outside it. The boundary is designed, documented, and defended — not discovered after the fact. Code crossing into Verity Consensus is held to its standard; code outside it exists to feed it clean, well-typed inputs and to carry its outputs to the network.

## Design principles

These follow directly from the beliefs above. Each is stated as a stance, with the reason it serves verifiability.

- **Pure, deterministic Verity Consensus.** The state transition and fork choice are expressed as pure functions of their inputs, free of hidden state, clocks, locks, or scheduling. Concurrency and input/output are pushed to the edges of the system. A proof can only reason about a deterministic function; the moment consensus logic depends on shared mutable state or task ordering, it stops being something we can fully verify. Keeping Verity Consensus pure is what makes it provable at all.

- **Explicitness over cleverness.** Verity prefers concrete, named types and straightforward control flow over macro-generated polymorphism and deep generic hierarchies. The implementation should read like the model it corresponds to, so that a reviewer — and a proof — can see the exact shape of every value and the exact obligation at every step. Cleverness that hides structure is a cost paid twice: once when reasoning about the code, and again when aligning it with the Lean 4 model.

- **Panic-freedom as a proof contract.** A proof about Verity Consensus is worthless if the surrounding program can still abort on unexpected input. Within the verified paths, fallible operations return explicit results rather than crashing, and the runtime is built so that it cannot diverge from the behavior the proof describes. Abrupt termination is treated as a correctness failure, not as an acceptable last resort handled elsewhere.

- **Arithmetic that mirrors proven invariants.** Numeric operations in the consensus layer respect the same bounds that the Lean 4 model proves. Overflow, underflow, and truncation are not runtime surprises to be caught downstream; they are conditions the model rules out, and the implementation is written so that its arithmetic corresponds one-to-one with those guarantees.

- **Reproducibility and supply-chain integrity are part of the proof.** A proof about source code says nothing about a binary that cannot be rebuilt from that source or whose dependencies cannot be audited. Verity treats reproducible builds and a disciplined, pinned, auditable dependency set as extensions of the correctness guarantee, not as operational afterthoughts. The chain of trust runs from the specification, through the proof, to the artifact a validator runs — and no link in it is left implicit.

- **Conformance through shared evidence.** The same specification-derived test vectors feed both the Rust implementation and the Lean 4 model, so the two are exercised against identical inputs and the gap between them stays visible and small. Beyond fixtures, interoperating with other clients on live devnets is the most demanding test of all, and Verity treats passing that gauntlet as the real measure of conformance.

- **Post-quantum security is structural.** Verity's signatures are hash-based and its aggregation is proof-based, as the protocol requires; this is not an option to be bolted on but a load-bearing part of the design. Secret key material is handled as the sensitive, stateful resource it is — protected in memory and persisted safely — because for a client real validators stake on, getting this wrong is not negotiable.

## Alignment with Lean Ethereum

Verity does not invent its goals; it inherits them from Lean Ethereum and commits to them at the level of implementation.

- **Security hardening.** Lean Ethereum moves consensus to post-quantum, hash-based signatures. Verity treats that migration as structural, as described above.
- **Decentralization.** Lean Ethereum lowers the barrier to participation so that ordinary stakers can run a validator. Verity's emphasis on a small, reproducible, panic-free client serves the same end: a client that is realistic to run and to trust.
- **Rapid finality.** Lean Ethereum finalizes in a few slots through its decoupled, layered consensus. Verity implements those finality rules faithfully and treats their safety conditions as inviolable.
- **Minimalism.** Lean Ethereum keeps the specification small enough to verify. Verity keeps the *implementation* small enough to verify.

To these four pillars Verity adds a fifth that is its own: **provability**. Lean Ethereum makes a consensus layer that *can* be verified; Verity is the client that carries that verification through to running code.

## What Verity deliberately rejects

Stating what we will not do is as important as stating what we will, and the existing Rust clients make the trade-offs concrete.

- **Macro-generated fork polymorphism that hides structure.** Synthesizing many protocol variants from a single declaration is powerful, but the resulting types are hard to read, hard to analyze, and hard to align with a formal model. Verity prefers explicit definitions per concern and shared behavior expressed through clear abstractions.
- **Backwards-compatibility shims.** Lean Ethereum advances by clean breaks between devnets. Verity does not accumulate compatibility layers for superseded protocol versions; it tracks the current specification and lets old shapes go.
- **Treating panics as acceptable.** Relying on a global safety net to absorb crashes is incompatible with proving that Verity Consensus cannot crash. Verity does not adopt that posture.
- **Floating or unaudited dependencies.** Unpinned versions and unreviewed upstreams break the chain of trust between source and artifact. Verity does not accept them.
- **Optimization that trades away verifiability.** Performance matters, but not at the price of consensus logic we can no longer reason about. Verity optimizes only behind the verification boundary or in ways that preserve the correspondence with the model, and only once a real bottleneck is shown.
- **Unbounded resource growth.** Queues and buffers without limits turn load into failure. Verity designs for explicit backpressure and bounded resource use rather than assuming the happy path.

## A note on KISS, SOLID, and the usual disciplines

These principles are assumed, not argued. Verity keeps functions small, separates concerns, and prefers the simple design — but it does so for a specific reason: simplicity and clear boundaries are what make the verification boundary defensible and the Lean 4 correspondence tight. When a generic best practice and a verifiability concern point in different directions, verifiability decides. The familiar disciplines are the means; a provable client is the end.

## Status

Verity is pre-implementation. This document is the north star that the architecture, the crate layout, and every future line of code must answer to. Both Lean Consensus and its reference implementation are still in motion — the protocol's design is under active discussion and leanSpec changes with it — so this document is expected to evolve as the specification advances and as the Lean 4 verification effort teaches us where the real boundaries lie. Its central commitment does not change: the running client is shown to match the specification, not merely believed to.
