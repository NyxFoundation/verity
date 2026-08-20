---
name: leanMetrics
description: |
  Ground every observability/metrics question in leanMetrics — the authoritative
  standard for Prometheus-compatible metrics across Lean Ethereum consensus clients
  (github.com/leanEthereum/leanMetrics), always read from the latest remote main.
  leanMetrics fixes the metric name strings, Prometheus types, Histogram buckets, label
  names/enum values, and collection events that every compliant client must match;
  Verity's verity-metrics crate implements it. Use before implementing, reviewing, or
  answering anything about exposing or monitoring consensus-client metrics.
  Triggers: "leanMetrics", "lean_metrics", "leanMetricsを確認", "メトリクス", "metrics",
  "prometheus", "grafana", "observability", "観測", "dashboard", "verity-metrics", any
  "lean_*" metric name (e.g. "lean_head_slot", "lean_block_aggregated_payloads"), and
  any work exposing or monitoring consensus-client metrics in Verity.
  Negative triggers: Do NOT activate for consensus container shapes / fork choice /
  state transition behavior (use the leanSpec skill). Do NOT activate for aggregation /
  zkVM / proof internals (use the leanVM skill). Do NOT activate for pure docs-site
  (mdBook) work. Do NOT activate when working outside the Verity project.
---

# leanMetrics — source of truth for observability metrics

leanMetrics (`github.com/leanEthereum/leanMetrics`) is the authoritative **standard**
for Prometheus-compatible metrics across all Lean Ethereum consensus clients. It is a
*specification (`metrics.md`) + Grafana dashboards (`dashboards/`)* — **not a library**.
It fixes what every compliant client must expose so that one dashboard serves all
clients. Verity's `verity-metrics` crate implements it.

## 0. Core principle

- leanMetrics governs the **exposed contract**: the metric **name string** (`lean_*`),
  the **Prometheus type** (Gauge/Counter/Histogram), the **Histogram bucket boundaries**,
  the **label names + enum values**, and the **"Sample collection event"** (where the
  client updates the metric). A compliant client matches these **exactly** — that match
  is the entire point: it lets a single Grafana dashboard work across every client.
- It is an **interop / observability standard, NOT consensus-critical** (unlike
  leanSpec). A wrong metric breaks dashboards, not the chain.
- **Internal code identifiers are free**; only the registered string + type + buckets +
  labels are governed. (Proof: ethlambda names its variable `LEAN_HEAD_SLOT`, Ream names
  its `HEAD_SLOT`, and both register the same string `"lean_head_slot"`.)
- The repo holds `metrics.md` (the spec table) and `dashboards/` (Grafana JSON) — no
  client instrumentation code. "Implementing leanMetrics" means writing matching
  Prometheus instrumentation **in the client**; "using the dashboards" means pointing
  Grafana at any compliant client.

## 1. Always read leanMetrics from the remote `main` (no local path assumptions)

leanMetrics evolves as clients add metrics. A stale local checkout is **not** the
standard. Read the latest `main` of the canonical upstream directly from the remote —
this works for every developer and in CI, with no dependency on anyone's local clone.

Repo: `github.com/leanEthereum/leanMetrics` (canonical; do **not** use a personal fork).

```bash
# Latest main commit — cite this SHA in your output:
gh api repos/leanEthereum/leanMetrics/commits/main --jq '.sha'

# Read the spec table at main:
gh api "repos/leanEthereum/leanMetrics/contents/metrics.md?ref=main" --jq '.content' | base64 -d
# raw fallback (no gh):
curl -s https://raw.githubusercontent.com/leanEthereum/leanMetrics/main/metrics.md

# List the dashboards at main:
gh api "repos/leanEthereum/leanMetrics/contents/dashboards?ref=main" --jq '.[].name'
# Full tree:
gh api "repos/leanEthereum/leanMetrics/git/trees/main?recursive=1" --jq '.tree[].path'
```

No `gh`/`curl`? Use WebFetch on `https://github.com/leanEthereum/leanMetrics/blob/main/<path>`.

If you happen to have a local clone, you *may* use it for fast navigation/grep — but
`git fetch origin` first and read `origin/main` (clones drift onto feature branches and
forks). Never assume a specific clone path.

## 2. What the spec defines per metric

`metrics.md` is a set of tables, one per category. Each row defines a metric via these
columns:

- **Name** — the exact `lean_*` string to register.
- **Type** — `Gauge` | `Counter` | `Histogram`.
- **Usage** — what it measures (use as the metric HELP text).
- **Sample collection event** — *when* the client updates it (e.g. "On state
  transition", "On block production", "On scrape"). This tells you where to instrument.
- **Labels** — label names and their allowed enum values (e.g. `result=success,error`).
- **Buckets** — for Histograms, the exact bucket boundaries (seconds, bytes, or counts).
- **Per-client status** — one column per tracked client.

Status legend: ✅ implemented / 📝 in progress / □ not implemented.

Tracked clients (status columns): **EthLambda, Grandine, Lantern, Lighthouse, Nlean,
Peam, Qlean, Ream, Zeam**.

What must match the spec vs what is free:

| Aspect | Must match leanMetrics? |
|---|---|
| Registered metric name string (`lean_*`) | ✅ required |
| Prometheus type (Gauge/Counter/Histogram) | ✅ required |
| Histogram bucket boundaries | ✅ required (and the unit: seconds/bytes/count) |
| Label names + enum values | ✅ required |
| Internal code identifier (variable name) | ❌ free |

## 3. Topic → authoritative location map

Paths are relative to the leanMetrics repo root, read at `main`. The implementation
column points at Rust references only (how clients have instrumented these) — it is not
authoritative for the contract.

| Topic (category) | leanMetrics (authoritative) | implementation reference |
|---|---|---|
| Node Info | `metrics.md#node-info-metrics` | ethlambda `crates/blockchain/src/metrics.rs`; Ream `crates/common/metrics/src/lib.rs` |
| PQ Signature | `metrics.md#pq-signature-metrics` | ethlambda `crates/blockchain/src/metrics.rs`; Ream `crates/common/metrics/src/lib.rs` |
| Block Production | `metrics.md#block-production-metrics` | (all clients □ — not yet implemented anywhere) |
| Fork-Choice | `metrics.md#fork-choice-metrics` | ethlambda / Ream metrics crates (as above) |
| State Transition | `metrics.md#state-transition-metrics` | ethlambda / Ream metrics crates (as above) |
| Validator | `metrics.md#validator-metrics` | ethlambda / Ream metrics crates (as above) |
| Network | `metrics.md#network-metrics` | ethlambda / Ream metrics crates (as above) |
| Grafana dashboards | `dashboards/lean-ethereum-clients-dashboard.json` | Ream `metrics/` (compose + Grafana stack) |

## 4. How to use (implementing / reviewing / answering)

1. Read the authoritative `metrics.md` category section at `main` first.
2. Match the name string, type, Histogram buckets (and their unit), and label
   names + enum values **exactly**; internal variable names are free.
3. For coverage/status questions, read the per-client status column (✅/📝/□).
4. In your output, cite the **`metrics.md` section + the `main` commit SHA** you checked.

## 5. Relationship to leanSpec & leanVM & Verity

- **leanSpec** (separate skill) = consensus protocol & container shapes (Python).
- **leanVM** (separate skill) = aggregation + zkVM (Rust).
- **leanMetrics** (this skill) = the observability **metric contract** (Prometheus
  names/types/buckets/labels) + Grafana dashboards.
- Verity's `verity-metrics` crate implements leanMetrics; **ethlambda** and **Ream** are
  Rust implementation references. The client is pre-implementation today.
