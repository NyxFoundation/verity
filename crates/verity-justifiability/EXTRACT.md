# Extraction slice: `is_justifiable_after`

Verification branch only. Not merged to `develop`.

This crate copies `verity-chain::justification::is_justifiable_after` so Charon
can see a crate that contains nothing else. Production logic is unchanged: the
body is the same, `Slot` is the same `u64` newtype without SSZ.

This branch drives Charon and Aeneas directly, not via `cargo hax`.

```
cd crates/verity-justifiability
cargo test -p verity-justifiability
./extract.sh
```

The Charon and Aeneas binaries default to the ones `cargo-hax` 0.4.0 pins
(`charon` nightly-2026.09.02, `aeneas` nightly-2026.09.03-6852e64). Install
them with `cargo hax tools install` if they are not already in
`~/.cache/hax/tools/`.

Generated Lean is under `proofs/lean/`. The LLBC is gitignored.
