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

## Host build requirements

Beyond the Rust toolchain, which `rust-toolchain.toml` pins and `rustup` installs on its own,
a Rust build needs two things from the host. Both come from `verity-db`'s RocksDB dependency,
which is compiled from source rather than linked against a system library:

- **A C++ toolchain.** `librocksdb-sys` builds RocksDB itself, plus `lz4-sys`.
- **libclang.** `librocksdb-sys` generates its FFI bindings with bindgen on every build, in
  `bindgen-runtime` mode, which loads `libclang` dynamically. On Debian and Ubuntu this is
  `libclang-dev`; CI runner images already carry it.

The first Rust build after a clean checkout therefore takes several minutes longer than the
crate count suggests, and is cached afterwards.

If bindgen reports `couldn't find any valid shared libraries matching: ['libclang.so',
'libclang-*.so']` while `libclang` is plainly installed, the library is present only under a
versioned name that bindgen's search does not match. Point it at one it does:

```bash
mkdir -p ~/.local/lib/libclang
ln -sf /usr/lib/llvm-19/lib/libclang-19.so.1 ~/.local/lib/libclang/libclang.so
export LIBCLANG_PATH=~/.local/lib/libclang
```

## Tooling versions

Hook versions are pinned by `rev` in `.pre-commit-config.yaml` and kept current by
Dependabot (the `pre-commit` ecosystem in `.github/dependabot.yml`). Lint settings
live in the existing config files (`_typos.toml`, `.markdownlint.jsonc`, `rustfmt.toml`)
— the pre-commit config does not duplicate them.
