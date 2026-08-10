# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project state

Verity is a formally verified Ethereum consensus client to be written in **Lean 4**, by Nyx Foundation. The project is **pre-implementation**: there is no Lean or Rust source yet. The only working component today is the **mdBook documentation site** under `docs/`. Most pages in `docs/src/` are intentional placeholders that list "Planned topics" — treat them as a content roadmap, not finished docs.

The intended architecture (per the docs and README references) targets the [Lean Consensus specification](https://github.com/leanEthereum/leanSpec) and the [lean roadmap](https://leanroadmap.org/), including post-quantum signatures, with the verified Lean core compiled via Lean's C backend into a static library and consumed by the Rust runtime over a C ABI (no Aeneas). The Lean side already exists: [NyxFoundation/formal-leanSpec](https://github.com/NyxFoundation/formal-leanSpec) holds the Lean 4 model and its proposition catalog, and Verity Consensus is defined as that model's compiled, exported subset (see `docs/src/concepts/formal-verification.md`). The Rust runtime is not implemented yet — verify against actual code before treating any of it as present.

This repository is a **monorepo**, not a docs-only repo: the Rust implementation will live here alongside `docs/`.

## Implementation decisions (2026-07-22, pre-scaffold)

Owner-ratified ground rules for the first Rust code. Do not re-open these without an explicit owner conversation.

- **Strategy**: Rust-first. Everything runs in Rust initially; Lean-compiled logic is adopted per-logic later (stable / proved / measured-within-budget first; STF and fork choice last, as they track a volatile upstream spec).
- **Goal**: a full node from the start (networking included), not a fixtures-passing library. leanSpec fixture conformance is still the CI backbone.
- **Crates**: start with a single `verity-consensus` crate (no chain/validator split). Proposer selection lives chain-side as a pure function next to STF/fork choice. Shared types stay a module until a second crate exists.
- **Dependencies**:
  - XMSS: [`leansig`](https://github.com/leanEthereum/leanSig) as a git dependency, **rev-pinned** (not branch-tracked), for per-validator sign / verify. Aggregation and aggregate-proof verification come from [`leanMultisig`](https://github.com/leanEthereum/leanMultisig) — both sit behind `verity-crypto`'s one capability contract, so a doc naming only one of them is incomplete, not contradictory.
  - SSZ: `libssz` 0.2.2 (lambdaclass). [NyxFoundation/leanSSZ](https://github.com/NyxFoundation/leanSSZ) (proven Lean SSZ, C ABI PoC complete) is deliberately NOT adopted initially — it is the future Lean-adoption candidate for SSZ.
  - Networking: upstream `rust-libp2p` (QUIC, gossipsub, reqresp). Fork only if a concrete need materializes, as lambdaclass did for ethlambda.
  - Storage: **RocksDB** behind a backend trait with an in-memory sibling implementation (ethlambda's `StorageBackend` split). Decided 2026-08-10 — see **Storage** below.
  - **leanSig pins Plonky3 to nothing — `Cargo.lock` is load-bearing here.** leanSig's manifest declares `p3-* = { git = ".../Plonky3.git" }` with neither rev nor branch, so a fresh resolve floats to Plonky3's default-branch HEAD. That HEAD (v0.6.0) moved to `num-bigint` 0.5 while leanSig's code is written against 0.4, so leanSig stops compiling with no change on our side. Our lockfile therefore pins Plonky3 to `3f67d136c71bec40f180c85d0bb2b654acddef22` (v0.5.1) — the rev ethlambda locks against the same leanSig rev.
    - `[patch]` does **not** work for this: cargo rejects a patch resolving to the same source, and a patch pointing at a textually different URL is silently dropped as "not used in the crate graph" because the pinned v0.5.1 cannot satisfy the v0.6.0 intra-workspace requirements. Use `cargo update p3-field --precise <rev>`, which moves every crate from that git source at once.
    - Consequence: never run a bare `cargo update`. It refloats Plonky3 to HEAD and breaks the build. The failure is loud (a `num_bigint` type mismatch inside leanSig), not silent — if you see it, this is why.
  - Both `leanSig` and `Plonky3` are unpublished git sources, so `deny.toml` lists them under `[sources] allow-git`. That entry allows the *host*; the leanSig rev is pinned in `Cargo.toml` and the Plonky3 rev in `Cargo.lock`.
- **Storage** (2026-08-10): aggregate proofs are persisted and pruned on a **21,600-slot (~1 day)** window, in their own table keyed `slot ‖ root` so pruning is a slot-ordered range delete. Two facts fix this:
  - Measured from leanSpec's `fixtures-prod-scheme.tar.gz`: an aggregate block proof is 155–236 KB (median 190 KB) against ~100–800 B for everything else. At 4 s slots that is ~4.1 GB/day of proofs vs ~5 MB/day of blocks and states — the large-value, bulk-delete workload is what selects an LSM engine.
  - leanSpec sets `MIN_SLOTS_FOR_BLOCK_REQUESTS = 3600` (4 hours) and a `BlocksByRange` responder MUST serve that window. One day is an operational choice 6× above that floor: it is how far a peer can fall behind and still catch up over P2P instead of needing a checkpoint. leanSpec's reference node meets the requirement in memory and persists no proofs at all; we persist so the guarantee survives a restart.
  - Do not re-derive these numbers from the devnet-scheme fixtures in the leanSpec source tree — those are much smaller (XMSS signature 424 B there vs 2,536 B in the production scheme) and will mislead.
- **Differential testing**: consume leanSpec's release asset `fixtures-prod-scheme.tar.gz` in CI, pinned to a commit and bumped manually. Use [leansig-test-keys](https://github.com/leanEthereum/leansig-test-keys) pre-generated keys for fast tests.
- **Toolchain**: Rust edition 2024, resolver 3, latest stable pinned via `rust-toolchain.toml` (external floor: leanSig requires ≥1.87; no nightly needed).
- **License**: MIT (Nyx Foundation copyright).
- **Devnet**: always track the latest devnet generation; never hardcode a generation in docs or code comments.
- **Verification harness**: NOT wired in from day one (no bolero/proptest in the initial scaffold or CI); introduced later per `MODEL_CHECK.md`'s tool-to-zone mapping.
- Known caveat: leanSig internally depends on `ethereum_ssz`, so two SSZ implementations coexist transitively — harmless, but mind type conversions at the signature boundary.

## Documentation site (`docs/`)

The docs are an [mdBook](https://rust-lang.github.io/mdBook/). All commands run from the `docs/` directory.

```bash
# CI pins these exact versions — match them locally to avoid drift.
# mdbook-mermaid 0.16.2 is the newest release compatible with mdBook 0.4.40
# (0.17.0 targets the mdBook 0.5 JSON protocol and fails to parse).
cargo install mdbook --version 0.4.40
cargo install --locked mdbook-mermaid --version 0.16.2

cd docs           # preprocessors use paths relative to docs/ — always build from here
mdbook build    # outputs to docs/book/ (gitignored)
mdbook serve     # live-reload preview at http://localhost:3000
```

- `docs/book.toml` — mdBook config (title, theme `navy`, GitHub edit links pointing at `NyxFoundation/verity`).
- `docs/src/SUMMARY.md` — the table of contents. **Every page must be registered here** or mdBook will not render it.
- `docs/preprocessors/strip-frontmatter.py` — mdBook preprocessor (needs `python3` on PATH) that strips the required YAML frontmatter before rendering, so it never shows on the published site.
- `docs/mermaid.min.js`, `docs/mermaid-init.js` — vendored mdbook-mermaid assets; regenerate with `mdbook-mermaid install .` when bumping mdbook-mermaid.
- `docs/wrangler.toml` — serves the built `book/` as Cloudflare Workers static assets (`verity-docs`).

### Editing docs frontmatter

Per global rules, every `.md` under `docs/` requires YAML frontmatter (`title`, `last_updated`, `tags`) — except `docs/generated/` and `docs/vendor/`. The existing `docs/src/` pages predate this rule and lack it; **add frontmatter when you next edit a page**, and include it in any new page from creation.

## CI / deployment

`.github/workflows/docs.yml` runs only when `docs/**` or the workflow file changes:

- **build** (PRs + pushes to `main`): `mdbook build` with mdBook `0.4.40`, uploads the site as an artifact.
- **deploy** (push to `main` only): downloads the artifact and deploys to Cloudflare Workers (`docs.verityclient.com`) via `wrangler deploy`. Requires `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` secrets.

Because the workflow is path-filtered, changes outside `docs/` do not trigger it.
