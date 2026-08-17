---
name: leanSpec
description: |
  Ground every Verity spec question in leanSpec — the single authoritative specification
  (Python reference implementation), always read from the latest origin/main (lstar HEAD).
  Use before implementing, reviewing, or answering anything about Verity's protocol
  behavior, container shapes, constants, fork choice, state transition, signatures, or
  test vectors. Maps a topic to the authoritative leanSpec path for any protocol element.
  Triggers: "leanSpec", "leanSpecを確認", "仕様を確認", "Verityの仕様", "prime", "verity context",
  "where is the spec for", "container shape", "コンテナの形", "3SF", "fork choice",
  "state transition", "XMSS", "leanVM", "devnet spec", and starting any Verity
  implementation or review work.
  Negative triggers: Do NOT activate for pure docs-site work (mdBook page authoring or
  build/serve). Do NOT activate when working outside the Verity project.
---

# leanSpec — single source of truth for Verity

Verity is a Rust Lean Consensus client. **leanSpec** (the Python reference implementation)
is the *only* authoritative specification. This skill tells you where the authoritative
definition of any protocol element lives, and how to read it correctly.

## 0. Core principle

- Container shapes, field order, constants, protocol behavior, and conformance test
  vectors are defined by **leanSpec** (`lstar` fork). Verity (Rust) must match it exactly.
- The user-facing `docs/` mdBook provides framing but **does not override leanSpec.**
  When they conflict, leanSpec wins — treat other docs as possibly lagging.
- Field order is consensus-critical (it determines `hash_tree_root`). Never reorder.

## 1. Always read leanSpec from the remote `main` (no local path assumptions)

Verity targets `lstar` HEAD (devnet-4/5 are in flight on `main`), so the spec moves.
A pinned commit or a stale local checkout is **not** the spec. Read the latest `main`
of the canonical upstream **`leanEthereum/leanSpec`** directly from the remote — this
works for every developer and in CI, with no dependency on anyone's local clone path.

Repo: `github.com/leanEthereum/leanSpec` (canonical; do **not** use a personal fork).

```bash
# Latest main commit — cite this SHA in your output:
gh api repos/leanEthereum/leanSpec/commits/main --jq '.sha'

# Read a file at main:
gh api "repos/leanEthereum/leanSpec/contents/src/lean_spec/forks/lstar/spec.py?ref=main" --jq '.content' | base64 -d
# raw fallback (no gh):
curl -s https://raw.githubusercontent.com/leanEthereum/leanSpec/main/src/lean_spec/forks/lstar/spec.py

# List a directory at main:
gh api "repos/leanEthereum/leanSpec/contents/src/lean_spec/subspecs?ref=main" --jq '.[].name'
# Full tree:
gh api "repos/leanEthereum/leanSpec/git/trees/main?recursive=1" --jq '.tree[].path'
```

No `gh`/`curl`? Use WebFetch on `https://github.com/leanEthereum/leanSpec/blob/main/<path>`.

If you happen to have a local clone, you *may* use it for fast navigation/grep — but
`git fetch origin` first and read `origin/main` (clones drift onto feature branches and
forks). Never assume a specific clone path.

## 2. Devnet pin awareness

- `origin/main` is the spec source of truth. `VERSIONS.md` records devnet commit pins
  (Devnet 3 = `be853180d21aa36d6401b8c1541aa6fcaad5008d`; devnet-4/5 not yet pinned —
  they live on `main`).
- Only consult a pin when checking interop against an already-shipped network (e.g. a
  devnet-3 build target). The final container shapes are governed by `main`.

## 3. Topic → authoritative location map

Paths are relative to the leanSpec repo root, read at `origin/main`.

| Topic | leanSpec (authoritative) |
|---|---|
| Container types / shapes | `src/lean_spec/forks/lstar/containers/` (`attestation/`, `block/`, `state/`, `validator.py`, `config.py`) |
| State transition | `src/lean_spec/forks/lstar/spec.py` |
| Fork choice | `src/lean_spec/forks/lstar/store.py`, `src/lean_spec/subspecs/forkchoice/` |
| XMSS signatures | `src/lean_spec/subspecs/xmss/` |
| Poseidon / KoalaBear (XMSS internals) | `src/lean_spec/subspecs/poseidon1/`, `poseidon2/`, `koalabear/` |
| SSZ / merkleization | `src/lean_spec/subspecs/ssz/` |
| Networking (gossipsub / req-resp) | `src/lean_spec/subspecs/networking/` |
| Storage | `src/lean_spec/subspecs/storage/` |
| Sync | `src/lean_spec/subspecs/sync/` |
| Genesis / validator / chain config | `src/lean_spec/subspecs/{genesis,validator,chain}/`, `forks/lstar/containers/config.py` |
| API | `src/lean_spec/subspecs/api/` |
| Conformance test vectors | `tests/consensus/devnet/{state_transition,fc,ssz,networking,sync,verify_signatures}/` |
| leanVM (aggregation / zkVM) | **not** in leanSpec — use the dedicated `leanVM` skill (`github.com/leanEthereum/leanVM`) |
| ethlambda (Rust *design* reference, not spec) | external `github.com/lambdaclass/ethlambda` |

## 4. How to use (implementing / reviewing / answering)

1. Read the **authoritative leanSpec path at `origin/main`** for the topic first.
2. Never let the user-facing `docs/` override leanSpec.
3. In your output, cite the **leanSpec path + the `origin/main` commit** you checked.

## 5. Verity layout reminder

- `docs/` — user-facing mdBook documentation (deployed to docs.verityclient.com).
- Verity's Rust crates mirror ethlambda's components (`verity-types`, `verity-fork-choice`,
  `verity-state-transition`, `verity-crypto`, `verity-p2p`, `verity-rpc`, `verity-storage`,
  `verity-metrics`, `verity-cli`). The client is pre-implementation today.
