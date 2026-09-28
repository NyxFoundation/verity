# Extraction of `verity-chain` pure functions

Verification branch only. Not merged to `develop`.

Extracts production `verity-chain` through Charon then Aeneas.

```
cd crates/verity-chain
./extract.sh
```

A failure is classified before any Rust is edited:

- **Tool limitation** — record it, leave the implementation alone.
  Getting the extractor to accept the file is not the goal.
- **Verity defect** — fix it on this branch.

## What extracts

`justification`, `proposer`, `slot_clock`, and `merkle`.

Generated Lean is under `proofs/lean/`. The LLBC is gitignored.

`lakefile.toml` / `lean-toolchain` pin Aeneas `nightly-2026.09.03` and
Lean 4.31.0. Config constants in `FunsExternal.lean` are the transcribed
literals, not axioms. `extract.sh` fails closed if Charon or Aeneas is
missing.

`is_justifiable_after : bool` extracts as `RustM Bool` because Aeneas
models `u64` arithmetic as overflow-fallible. That is the extractor's
Rust semantics, not a Verity defect.

## CONT-2 correspondence

The Lean package pins formal-leanSpec at `ba7284513031eac5c66bfb8221d27b6154cf240b`.
`VerityChain.Correspondence` proves three properties about the generated Rust
semantics:

- `isJustifiableAfter_eq`: at or after finalization, the extracted `RustM Bool`
  succeeds with formal-leanSpec's `Slot.isJustifiableAfter` result;
- `isJustifiableAfter_iff`: the extracted predicate directly satisfies CONT-2:
  the distance from finalization is at most 5, a perfect square, or a pronic
  number exactly when the predicate succeeds with `true`;
- `isJustifiableAfter_before_finalized`: a slot before finalization succeeds
  with `false`.

The hand-written external model implements Rust's `u128::isqrt` with
formal-leanSpec's proved `Slot.isqrt`; the correspondence theorems use no
project-specific axiom or `sorry`. `lake build` checks both generated code and
the correspondence after every extraction:

```
./extract.sh
cd proofs/lean
lake build
```

Generated modules live under `proofs/lean/VerityChain/`; `extract.sh` restores
that Lake module layout without overwriting the hand-written external model or
correspondence proof.

## Tool limitations (do not rewrite to pass)

The surfaces below were attempted on the sibling hax branch. The same
extractor subset applies here; this script does not probe them again.

| Surface | What failed | Why it is a tool limit |
|---|---|---|
| `state_transition` | closures; "Can't end abstraction" | Aeneas does not functionalize this iterator/closure/borrow shape |
| `fork_choice` | `HashMap` / `Iterator` associated types | Charon type error inside `std` |
| `block_production` | `return` inside a nested loop | Documented Aeneas gap |
