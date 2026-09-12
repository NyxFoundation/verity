# Extraction of `verity-chain` pure functions

Verification branch only. Not merged to `develop`.

Extracts production `verity-chain` through Charon then Aeneas. No slice
crate, no logic rewrite.

```
cd crates/verity-chain
./extract.sh
```

## What extracts

`justification`, `proposer`, `slot_clock`, and `merkle`.

Generated Lean is under `proofs/lean/`. The LLBC is gitignored.

## What Charon / Aeneas refused

Attempted on the sibling hax branch and left out, without rewriting
production code:

- `state_transition` — closures and a non-endable borrow abstraction
- `fork_choice` — `HashMap` / iterator trait matching
- `block_production` — `return` inside a nested loop
