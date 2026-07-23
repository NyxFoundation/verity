---
name: leanMultisig
description: |
  Ground every question about post-quantum signature aggregation and the zkVM in
  leanMultisig — the authoritative spec and Rust reference implementation
  (github.com/leanEthereum/leanMultisig), always read from the latest remote main.
  Verity's verity-crypto crate depends on it directly. Use before implementing,
  reviewing, or answering anything about XMSS aggregation, Type-1/Type-2 proofs, the
  prover/verifier, the zkVM, the zkDSL, or WHIR.
  Triggers: "leanMultisig", "lean_multisig", "leanMultisigを確認", "署名集約", "aggregation",
  "Type-1 proof", "Type-2 proof", "zkVM", "zkDSL", "WHIR", "prover", "verifier",
  "verity-crypto", and any work touching signature aggregation or proof generation in Verity.
  Negative triggers: Do NOT activate for consensus container shapes / fork choice / state
  transition (use the leanSpec skill). Do NOT activate for pure docs-site work. Do NOT
  activate when working outside the Verity project.
---

# leanMultisig — source of truth for aggregation & the zkVM

leanMultisig (`github.com/leanEthereum/leanMultisig`) is a minimal zkVM targeting
aggregation of hash-based (Generalized-XMSS) signatures. It is the **authoritative spec
and Rust reference implementation** for everything aggregation/zkVM-related in Verity.
Verity's `verity-crypto` crate depends on it directly (as a pinned git dependency).

## 0. Core principle

- The behavior of signature aggregation, Type-1/Type-2 proofs, the prover/verifier, the
  zkVM and its zkDSL is defined by **leanMultisig** (its Rust crates + design docs). When
  in doubt, read the source.
- Division of authority with the leanSpec skill: **leanSpec** defines the *consensus
  containers* that carry proofs (`TypeOneMultiSignature`, `SignedBlock.proof` =
  `ByteList512KiB`, the verify entry points). **leanMultisig** defines how those proofs
  are *produced and verified*. For container shapes → leanSpec; for proof internals →
  here.

## 1. Always read leanMultisig from the remote `main` (no local path assumptions)

leanMultisig moves fast. Read the latest `main` of the canonical upstream directly from
the remote — this works for every developer and in CI, with no dependency on anyone's
local clone path.

Repo: `github.com/leanEthereum/leanMultisig` (canonical; do **not** use a personal fork).

```bash
# Latest main commit — cite this SHA in your output:
gh api repos/leanEthereum/leanMultisig/commits/main --jq '.sha'

# Read a file at main:
gh api "repos/leanEthereum/leanMultisig/contents/crates/xmss/xmss.md?ref=main" --jq '.content' | base64 -d
# raw fallback (no gh):
curl -s https://raw.githubusercontent.com/leanEthereum/leanMultisig/main/crates/xmss/xmss.md

# List a directory at main:
gh api "repos/leanEthereum/leanMultisig/contents/crates?ref=main" --jq '.[].name'
# Full tree:
gh api "repos/leanEthereum/leanMultisig/git/trees/main?recursive=1" --jq '.tree[].path'
```

No `gh`/`curl`? Use WebFetch on `https://github.com/leanEthereum/leanMultisig/blob/main/<path>`.

If you happen to have a local clone, you *may* use it for fast navigation/grep — but
`git fetch origin` first and read `origin/main` (clones drift onto feature branches and
forks). Never assume a specific clone path.

## 2. How Verity consumes it

- `verity-crypto` depends on leanMultisig as a **pinned git dependency** in `Cargo.toml`.
  The pinned commit is what Verity builds against; the latest `main` is the source of
  truth for understanding/spec questions. When bumping the pin, re-read `main`.
- The proving stack is expensive; the prover context is set up once at startup. Verity
  produces Type-1 proofs only when acting as an aggregator; every node verifies.

## 3. Topic → authoritative location map

Paths are relative to the leanMultisig repo root, read at `main`.

| Topic | leanMultisig (authoritative) |
|---|---|
| XMSS signature scheme | `crates/xmss/` (+ `crates/xmss/xmss.md`) |
| Aggregation: Type-1 / Type-2 proofs | `crates/rec_aggregation/` (+ `crates/rec_aggregation/TYPE1_TYPE2_LAYOUT.md`) |
| Prover | `crates/lean_prover/` |
| zkVM | `crates/lean_vm/` |
| zkDSL compiler | `crates/lean_compiler/` (+ `crates/lean_compiler/zkDSL.md`) |
| WHIR (polynomial commitment) | `crates/whir/` |
| Proof backend (AIR, sumcheck, Fiat-Shamir, field, KoalaBear, poly, symmetric) | `crates/backend/` (`air`, `sumcheck`, `fiat-shamir`, `field`, `koala-bear`, `poly`, `symetric`, …) |
| Sub-protocols | `crates/sub_protocols/` |
| Top-level crate / public API | `src/lib.rs`, `Cargo.toml` |
| Tests | `tests/` |
| Overview / design | `README.md`, `minimal_zkVM.pdf`, `TODO.md` |

## 4. How to use (implementing / reviewing / answering)

1. Read the **authoritative leanMultisig path at `main`** for the topic first.
2. For the consensus containers carrying proofs, defer to the **leanSpec** skill.
3. In your output, cite the **leanMultisig path + the `main` commit SHA** you checked.

## 5. Relationship to leanSpec & Verity

- **leanSpec** (separate skill) = consensus protocol spec & container shapes (Python).
- **leanMultisig** (this skill) = aggregation + zkVM (Rust); produces/verifies the proofs
  that fill leanSpec's containers.
- Verity (Rust) wires both together: `verity-types` (leanSpec shapes) +
  `verity-crypto` (leanMultisig + Generalized-XMSS). The client is pre-implementation today.
