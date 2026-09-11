# Extraction slice: `is_justifiable_after`

Verification branch only. Not merged to `develop`.

This crate copies `verity-chain::justification::is_justifiable_after` so Charon
can see a crate that contains nothing else. Production logic is unchanged: the
body is the same, `Slot` is the same `u64` newtype without SSZ.

```
cd crates/verity-justifiability
cargo test --locked
cargo hax extract justifiable
```

Generated Lean is under `proofs/justifiable/lean/`.
