---
name: hive
description: |
  Ground every "build / run the hive E2E test harness" question in hive — the Ethereum
  end-to-end test harness that runs Dockerized clients against simulator test suites
  (github.com/ethereum/hive), always read from the latest remote master. It is
  operational tooling (Go core + Rust hivesim-rs), not a spec or library; the source of
  truth for HOW to run it is the upstream README + docs/commandline.md + simulator READMEs
  at master, which evolve. Especially relevant to Verity via the simulators/lean suite and
  the lean clients in clients/. Use before building, running, or answering anything about
  running hive (especially the lean simulator) for Verity.
  Triggers: "hive", "ethereum/hive", "hivesim", "hive simulator", "hive sim", "--sim lean",
  "simulators/lean", "lean simulator", "client interop", "E2E test", "結合テスト",
  "互換性テスト", and any work building or running the hive test harness (especially the
  lean simulator) for Verity.
  Negative triggers: Do NOT activate for spinning up a local devnet to RUN it rather than
  test it (use the leanQuickstart skill). Do NOT activate for consensus container shapes /
  fork choice / state transition (use the leanSpec skill). Do NOT activate for aggregation
  / zkVM / proof internals (use the leanVM skill). Do NOT activate for the metric
  contract — names/types/buckets/labels — (use the leanMetrics skill). Do NOT activate for
  pure docs-site (mdBook) work. Do NOT activate when working outside the Verity project.
---

# hive — operational runbook for the Ethereum E2E test harness

hive (`github.com/ethereum/hive`) is the **Ethereum end-to-end test harness**: it builds
each client as a Docker image, runs *simulator* test suites against them in isolated
containers, and reports a pass/fail matrix. It is **tooling** (Go core + a Rust
`hivesim-rs` framework), not a spec or a library. For Verity it matters because of the
`simulators/lean` suite and the lean clients already wired up in `clients/` — this is
where Verity would eventually be added as a client and tested for interop.

## 0. Core principle

- This skill grounds **how to build and run hive** — flags, simulator selection, client
  selection, reading results — not protocol semantics. Chain behaviour is leanSpec's
  domain; this is the harness that exercises clients.
- The authoritative source for *how to run it* is the upstream `README.md`,
  `docs/commandline.md`, and the target simulator's `README.md` at `master`. They change.
  **Do not quote flags or paths from memory or a stale clone** — read `master` and cite
  the SHA.
- **The default branch is `master`** (not `main`). License is GPL-3.0.

## 1. Always read hive from the remote `master` (no local path assumptions)

The harness evolves (sims and flags are added often). Read the latest `master` of the
canonical upstream directly from the remote.

Repo: `github.com/ethereum/hive` (canonical; default branch `master`).

```bash
# Latest master commit — cite this SHA in your output:
gh api repos/ethereum/hive/commits/master --jq '.sha'

# Read the command-line docs (authoritative for flags) at master:
gh api "repos/ethereum/hive/contents/docs/commandline.md?ref=master" --jq '.content' | base64 -d
# raw fallback (no gh):
curl -s https://raw.githubusercontent.com/ethereum/hive/master/docs/commandline.md

# Lean simulator README:
curl -s https://raw.githubusercontent.com/ethereum/hive/master/simulators/lean/README.md

# Full tree at master:
gh api "repos/ethereum/hive/git/trees/master?recursive=1" --jq '.tree[].path'
```

No `gh`/`curl`? Use WebFetch on `https://github.com/ethereum/hive/blob/master/<path>`.

If you happen to have a local clone, `git fetch origin` first and read `origin/master`.
Never assume a specific clone path.

## 2. Install & run (the canonical commands)

Prereqs (per `docs/commandline.md`): Linux, **Go ≥ 1.17**, and a working **Docker** setup
on the **same host** (hive talks to local `dockerd`; remote Docker is unsupported). Add
your user to the `docker` group to avoid `sudo`.

```bash
git clone https://github.com/ethereum/hive
cd hive
go build .

# Run a simulation (always from the repo root):
./hive --sim <simulation> --client <client[,client...]>
```

- `--sim <name>` — which simulator suite to run (e.g. `lean`, `ethereum`, `eth2`,
  `devp2p`, `portal`, `smoke`).
- `--client <list>` — comma-separated clients to test; pin a version with `_`
  (e.g. `go-ethereum_v1.9.23`).
- Results land in `workspace/logs/`; view them with the `hiveview` HTML viewer.

## 3. Running the lean simulator (Verity-relevant)

From `simulators/lean/README.md`:

```bash
./hive --sim lean --client-file simulators/lean/clients/devnet3.yaml --client ream
# devnet4 profile:
./hive --sim lean --client-file simulators/lean/clients/devnet4.yaml --client ream
```

- The active devnet is resolved from the **client name** (`ream_devnet4`,
  `ethlambda_devnet3`, `gean_devnet3`, …) and validated against the support matrix in
  `simulators/lean/config/lean-devnets.txt`.
- The lean sim currently runs **RPC-compat, sync, and client-interop** suites. Client
  interop runs each selected client against itself and against every other selected client
  in three-node 2:1 topologies, asserting all three finalize past genesis at the same slot.
- Post-genesis justification/finalization cases are driven by the `lean-spec-client`
  helper (it caches a fixed set of validator keys at image-build time).

## 4. Key flags (confirm the full set in `docs/commandline.md` at master)

- `--sim.limit <regex>` — select suites/tests; split at the first `/` (suite before,
  test after). E.g. `--sim.limit eth/Large`, or `--sim.limit /stBugs/` for any suite.
- `--client-file <yaml>` — client list with per-client `dockerfile`, `build_args`
  (`tag`, `baseimage`, `github`), and `nametag`.
- `--docker.pull` / `--docker.nocache <regex>` / `--docker.output` — image rebuild &
  output control (use `--docker.nocache` during simulator development).
- `--sim.timelimit <dur>` — abort the simulator after this time (no default).
- `--client.checktimelimit <dur>` — wait for the client RPC port (default 3m).
- `--sim.loglevel <0-5>` — client log verbosity (default 3).

## 5. Topic → authoritative location map

Paths are relative to the hive repo root, read at `master`.

| Topic | Path in ethereum/hive |
|---|---|
| Install / run / all flags | `docs/commandline.md`; `README.md` |
| Overview & concepts | `docs/overview.md` |
| Adding/configuring a client | `docs/clients.md`; `clients/<name>/` (Dockerfile, `hive.yaml`, `*.sh`, `mapper.jq`) |
| Writing simulators | `docs/simulators.md`; `hivesim/` (Go), `hivesim-rs/` (Rust) |
| Lean simulator | `simulators/lean/` (`README.md`, `src/scenarios/`, `clients/devnet*.yaml`, `config/lean-devnets.txt`) |
| Lean sim scenarios | `simulators/lean/src/scenarios/` (`client_interop`, `gossip`, `reqresp`, `rpc_compat`, `spec_assets`, `sync`, `validation`) |
| Other sims | `simulators/{ethereum,eth2,devp2p,portal,smoke}/` |
| Lean clients | `clients/{ethlambda,gean,grandine_lean,lantern,lean-spec-client,nlean,qlean,ream,zeam}/` |
| Hive core / CLI | `cmd/`, `internal/` |

## 6. How to use (building / running / answering)

1. Read `docs/commandline.md` and the target simulator's `README.md` at `master` first —
   never quote flags from memory.
2. Build with `go build .`; ensure Docker is running locally.
3. Run with the right `--sim` / `--client` (or `--client-file`); for lean, use the
   `simulators/lean/clients/devnet*.yaml` profiles.
4. Inspect `workspace/logs/` (hiveview) for results.
5. In your output, cite the **file/path + the `master` commit SHA** you checked.

## 7. Relationship to the other skills & Verity

- **leanQuickstart** (separate skill) = spin up and **run** a local lean devnet.
- **leanSpec** / **leanVM** / **leanMetrics** (separate skills) = protocol &
  container shapes / aggregation & zkVM / the metric contract.
- **hive** (this skill) = the **test harness** that exercises clients (build + `--sim` +
  `--client`), especially the `lean` simulator.
- Verity is pre-implementation, so it is **not yet a client** in hive's `clients/`.
