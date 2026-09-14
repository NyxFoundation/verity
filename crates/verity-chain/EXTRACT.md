# Extraction of `verity-chain` pure functions

Verification branch only. Not merged to `develop`.

Extracts production `verity-chain` through `cargo hax extract`.

```
cd crates/verity-chain
cargo hax extract
```

A failure is classified before any Rust is edited:

- **Tool limitation** — record it, leave the implementation alone.
  Getting the extractor to accept the file is not the goal.
- **Verity defect** — fix it on this branch.

## What extracts

| Scenario | Surface |
|---|---|
| `pure` | `justification`, `proposer`, `slot_clock` |
| `merkle` | `hash_tree_root` |

Generated Lean is under `proofs/<scenario>/lean/`.

`is_justifiable_after : bool` extracts as `RustM Bool` because Aeneas
models `u64` arithmetic as overflow-fallible. That is the extractor's
Rust semantics, not a Verity defect, and it is the gap against
formal-leanSpec's pure `Bool` (CONT-2).

## Tool limitations (do not rewrite to pass)

Attempted and refused. Classified as extractor subset, not implementation
bugs:

| Surface | What failed | Why it is a tool limit |
|---|---|---|
| `state_transition` | closures; "Can't end abstraction" | Aeneas does not functionalize this iterator/closure/borrow shape |
| `fork_choice` | `HashMap` / `Iterator` associated types | Charon type error inside `std` |
| `block_production` | `return` inside a nested loop | Documented Aeneas gap |
