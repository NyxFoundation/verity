# Contributing to Verity

## Branches

- `develop` is DEV and the default branch. Open pull requests against `develop`.
- `main` is PRD. Do not target it unless the change is a production hotfix.

## Local checks with pre-commit

Verity uses [pre-commit](https://pre-commit.com/) to run fast, deterministic lint
locally — before code reaches CI. The same `.pre-commit-config.yaml` is run by the
`pre-commit` job in GitHub Actions, so local and CI checks never drift apart.

### One-time setup

```bash
pipx install pre-commit        # or: pip install pre-commit / brew install pre-commit
pre-commit install             # wires up both pre-commit and pre-push hooks
```

`pre-commit install` activates both hook stages in one command
(`default_install_hook_types` is set in the config).

### Everyday use

Hooks run automatically on `git commit` (and `git push` for clippy). To run them by hand:

```bash
pre-commit run --all-files     # run every hook against the whole tree
pre-commit run typos           # run a single hook
```

If a hook auto-fixes a file (e.g. `cargo fmt`, `end-of-file-fixer`), re-stage the
change and commit again.

## What runs where

Local hooks give fast feedback; GitHub Actions is the enforcement gate that also
catches `--no-verify` bypasses and contributors who never ran `pre-commit install`.

| Check | Local (pre-commit) | GitHub Actions |
| --- | --- | --- |
| `trailing-whitespace`, `end-of-file-fixer`, `check-yaml`, `mixed-line-ending` | commit | `Quality / pre-commit` |
| `typos` (spell check) | commit | `Quality / pre-commit` |
| `markdownlint-cli2` (`docs/src`) | commit | `Quality / pre-commit` |
| `cargo fmt` | commit | `Rust / check` |
| `cargo clippy` | pre-push | `Rust / check` |
| `cargo test` / `build` | — | `Rust / check` |
| `cargo deny` | — | `Rust / deny` |
| secret scan (betterleaks) | — | `Secret Scan` |
| mdBook build / deploy | — | `Docs` |
| offline link check | — | `Docs / links` |
| online link check (weekly) | — | `Quality / link-check` |

The `Quality / pre-commit` job runs `pre-commit run --all-files` with `SKIP=fmt,clippy`:
the Rust hooks need the toolchain, so they run in the `Rust` workflow where the
toolchain and build cache live.

## Tooling versions

Hook versions are pinned by `rev` in `.pre-commit-config.yaml` and kept current by
Dependabot (the `pre-commit` ecosystem in `.github/dependabot.yml`). Lint settings
live in the existing config files (`_typos.toml`, `.markdownlint.jsonc`, `rustfmt.toml`)
— the pre-commit config does not duplicate them.
