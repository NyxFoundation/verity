# Extraction of `verity-chain` pure functions

Verification branch only. Not merged to `develop`.

Extracts production `verity-chain` through `cargo hax extract`. No slice
crate, no logic rewrite.

```
cd crates/verity-chain
cargo hax extract
```

## What extracts

| Scenario | Surface |
|---|---|
| `pure` | `justification`, `proposer`, `slot_clock` |
| `merkle` | `hash_tree_root` |

Generated Lean is under `proofs/<scenario>/lean/`.

## What Charon / Aeneas refused

Attempted and left out, without rewriting production code:

- `state_transition` — closures and a non-endable borrow abstraction
- `fork_choice` — `HashMap` / iterator trait matching
- `block_production` — `return` inside a nested loop
