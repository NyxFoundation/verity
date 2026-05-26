# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project state

Verity is a formally verified Ethereum consensus client to be written in **Lean 4**, by Nyx Foundation. The project is **pre-implementation**: there is no Lean or Rust source yet. The only working component today is the **mdBook documentation site** under `docs/`. Most pages in `docs/src/` are intentional placeholders that list "Planned topics" — treat them as a content roadmap, not finished docs.

The intended architecture (per the docs and README references) targets the [Lean Consensus specification](https://github.com/leanEthereum/leanSpec) and the [lean roadmap](https://leanroadmap.org/), including post-quantum signatures, with Lean proofs intended to integrate with a Rust runtime via Aeneas. None of this is implemented yet — verify against actual code before treating any of it as present.

## Documentation site (`docs/`)

The docs are an [mdBook](https://rust-lang.github.io/mdBook/). All commands run from the `docs/` directory.

```bash
# CI pins this exact version — match it locally to avoid drift
cargo install mdbook --version 0.4.40   # or: cargo binstall / OS package

cd docs
mdbook build    # outputs to docs/book/ (gitignored)
mdbook serve     # live-reload preview at http://localhost:3000
```

- `docs/book.toml` — mdBook config (title, theme `navy`, GitHub edit links pointing at `NyxFoundation/verity`).
- `docs/src/SUMMARY.md` — the table of contents. **Every page must be registered here** or mdBook will not render it.
- `docs/wrangler.toml` — serves the built `book/` as Cloudflare Workers static assets (`verity-docs`).

### Editing docs frontmatter

Per global rules, every `.md` under `docs/` requires YAML frontmatter (`title`, `last_updated`, `tags`) — except `docs/generated/` and `docs/vendor/`. The existing `docs/src/` pages predate this rule and lack it; **add frontmatter when you next edit a page**, and include it in any new page from creation.

## CI / deployment

`.github/workflows/docs.yml` runs only when `docs/**` or the workflow file changes:

- **build** (PRs + pushes to `main`): `mdbook build` with mdBook `0.4.40`, uploads the site as an artifact.
- **deploy** (push to `main` only): downloads the artifact and deploys to Cloudflare Workers (`docs.verityclient.com`) via `wrangler deploy`. Requires `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` secrets.

Because the workflow is path-filtered, changes outside `docs/` do not trigger it.
