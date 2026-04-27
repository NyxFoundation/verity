# Introduction

**Verity** is the Provable Consensus Client — a formally verified Ethereum consensus client built with [Lean 4](https://leanprover.github.io/) by [Nyx Foundation](https://github.com/NyxFoundation).

Where other clients test for correctness, Verity proves it. Every line of consensus logic is mathematically proven correct, closing the gap between specification and implementation — permanently.

> Verity is currently under active development and has not been released yet.

## Why Verity

- **Formally verified.** Specification drift is structurally impossible; entire bug classes are ruled out before the code ever runs.
- **Built in Lean 4.** A modern theorem prover and dependently typed language designed for both proofs and programs.
- **Aligned with the Ethereum roadmap.** Targets the Lean Consensus specification and post-quantum signature schemes.

## Where to go next

- [Overview](./getting-started/overview.md) — what Verity does and how the pieces fit together.
- [Formal Verification](./concepts/formal-verification.md) — what "proven correct" actually means here.
- [Architecture](./reference/architecture.md) — module map and crate layout.
