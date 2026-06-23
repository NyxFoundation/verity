---
name: leanQuickstart
description: |
  Ground every "spin up / run a local lean multi-client devnet" question in
  lean-quickstart — the utility for bootstrapping a localnet of Lean Ethereum
  multi-client nodes (github.com/blockblaz/lean-quickstart), always read from the
  latest remote main. It is operational tooling (Shell + Python + Ansible + Docker),
  not a spec or library; the source of truth for HOW to run it is the upstream
  README + scripts at main, which evolve. Use before running, configuring, or
  answering anything about starting a local Lean devnet for Verity.
  Triggers: "leanQuickstart", "lean-quickstart", "lean_quickstart", "localnet",
  "devnet", "ローカルネット", "デブネット", "spin-node", "spin-node.sh",
  "validator-config.yaml", "generate-genesis", "multi-client devnet", "マルチクライアント",
  and any work spinning up or running a local lean multi-client devnet for Verity.
  Negative triggers: Do NOT activate for consensus container shapes / fork choice /
  state transition (use the leanSpec skill). Do NOT activate for aggregation / zkVM /
  proof internals (use the leanMultisig skill). Do NOT activate for the metric
  contract — names/types/buckets/labels — (use the leanMetrics skill); this skill
  only covers RUNNING the bundled Prometheus/Grafana stack. Do NOT activate for pure
  docs-site (mdBook) work. Do NOT activate when working outside the Verity project.
---

# leanQuickstart — operational runbook for the local multi-client devnet

lean-quickstart (`github.com/blockblaz/lean-quickstart`) is "a utility to quickly spin up
a localnet of lean (multi-client) nodes". It is **tooling**, not a spec or a library:
Shell scripts orchestrate genesis generation, per-client launch commands, and an optional
Prometheus/Grafana metrics stack, driven by a single `validator-config.yaml`. Multiple
*different client implementations* (Zeam, Ream, Lighthouse, …) run against one shared
genesis so you can test interop on a single local chain.

## 0. Core principle

- This skill grounds **how to run the devnet** — flags, config, genesis, metrics — not
  protocol semantics. The chain behaviour is leanSpec's domain; this is the harness that
  starts nodes.
- The authoritative source for *how to run it* is the upstream `README.md` + the scripts
  at `main`. They change (the repo is actively developed). **Do not quote flags or
  filenames from memory or from a stale local clone** — read `main` and cite the SHA.
- "Run the devnet" = edit `validator-config.yaml`, then invoke `spin-node.sh`. Everything
  else (genesis, per-client commands, metrics) is wired from that config.

## 1. Always read lean-quickstart from the remote `main` (no local path assumptions)

The tooling evolves. Read the latest `main` of the canonical upstream directly from the
remote — this works for every developer and in CI, with no dependency on a local clone.

Repo: `github.com/blockblaz/lean-quickstart` (canonical; default branch `main`. Do **not**
use a personal fork.).

```bash
# Latest main commit — cite this SHA in your output:
gh api repos/blockblaz/lean-quickstart/commits/main --jq '.sha'

# Read the README (the authoritative quick-start) at main:
gh api "repos/blockblaz/lean-quickstart/contents/README.md?ref=main" --jq '.content' | base64 -d
# raw fallback (no gh):
curl -s https://raw.githubusercontent.com/blockblaz/lean-quickstart/main/README.md

# Read a specific script (e.g. the entry point) at main:
curl -s https://raw.githubusercontent.com/blockblaz/lean-quickstart/main/spin-node.sh

# Full tree at main:
gh api "repos/blockblaz/lean-quickstart/git/trees/main?recursive=1" --jq '.tree[].path'
```

No `gh`/`curl`? Use WebFetch on `https://github.com/blockblaz/lean-quickstart/blob/main/<path>`.

If you happen to have a local clone, you *may* use it for fast navigation/grep — but
`git fetch origin` first and read `origin/main` (clones drift onto feature branches and
forks). Never assume a specific clone path.

## 2. Quick start (the canonical command)

```bash
NETWORK_DIR=local-devnet ./spin-node.sh --node all --generateGenesis --popupTerminal
```

- `--node all` — launch **every node** defined in the config; `--node <name>` launches a
  single node by name.
- `--generateGenesis` — (re)generate genesis state, validator keys, and config from
  `validator-config.yaml` before starting (PQ hash-sig keys via `ethpandaops/eth-beacon-genesis`).
- `--popupTerminal` — open each node's logs in its own terminal window.
- `NETWORK_DIR` selects the network directory (`local-devnet` for local; `ansible-devnet`
  for the remote/Ansible flow).

The **authoritative, current flag set** lives in `spin-node.sh --help` and the README at
`main` — confirm there before relying on any flag, as options change.

## 3. Topic → authoritative location map

Paths are relative to the lean-quickstart repo root, read at `main`.

| Topic | Path in lean-quickstart |
|---|---|
| Main entry point / flags | `spin-node.sh`; README "Quick Start" |
| Initial host setup | `set-up.sh` |
| Genesis generation | `generate-genesis.sh` (PQ hash-sig keys via eth-beacon-genesis) |
| Network/node config (local) | `local-devnet/genesis/validator-config.yaml` |
| Network/node config (remote) | `ansible-devnet/genesis/validator-config.yaml` |
| Config expansion (subnets) | `convert-validator-config.py`, `generate-subnet-config.py` |
| Per-client launch commands | `client-cmds/<client>-cmd.sh` (zeam, ream, qlean, lantern, lighthouse, grandine, ethlambda, gean, nlean, peam) |
| Remote (Ansible) deploy | `ansible-deploy.sh`, `run-ansible.sh`, `ansible/` |
| Metrics stack (run it) | `generate-prometheus-config.sh`, `metrics/docker-compose-metrics.yaml`, `metrics/grafana/` |
| Devnet test notes / retros | `TESTING_DEVNET3.md`, `docs/devnets/` |

## 4. How to use (running / configuring / answering)

1. Read the README and `spin-node.sh --help` at `main` first — never quote flags from
   memory.
2. Edit `validator-config.yaml` (the single source of truth) to define which nodes, which
   client implementations, and how many validators each.
3. Run `spin-node.sh` with the appropriate `--node` / `--generateGenesis` flags; bring up
   `metrics/docker-compose-metrics.yaml` if you want the Prometheus/Grafana stack.
4. In your output, cite the **file/script + the `main` commit SHA** you checked.

## 5. Relationship to leanSpec, leanMultisig, leanMetrics & Verity

- **leanSpec** (separate skill) = consensus protocol & container shapes (the chain rules).
- **leanMultisig** (separate skill) = aggregation + zkVM proof internals.
- **leanMetrics** (separate skill) = the observability **metric contract** (names/types/
  buckets/labels). This skill only covers **running** the bundled Prometheus/Grafana
  stack, not what the metrics are.
- **leanQuickstart** (this skill) = the harness that **boots and runs** a local
  multi-client devnet.
- Verity is pre-implementation, so it is **not yet a node** in `validator-config.yaml`.
