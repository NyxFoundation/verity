---
title: Design Documents
last_updated: 2026-08-26
tags:
  - design
  - index
---

# Design Documents

Internal design records: the decisions behind Verity's implementation, the evidence
they rest on, and the leanSpec revision each was read against. They are not part of the
published mdBook — [docs.verityclient.com](https://docs.verityclient.com) carries the
reader-facing documentation instead.

**The architecture is not here.** It is published, and lives at
[`docs/src/reference/architecture.md`](../src/reference/architecture.md). Everything below
elaborates one axis of it.

| Document | What it settles |
|---|---|
| [Domain Model](domain-model.md) | The consensus entities, value objects, and aggregates as leanSpec defines them, mapped onto the verification zones |
| [Concurrency Model](concurrency.md) | Which primitive enforces the single-writer discipline, where verification executes, and how inbound work reaches the consensus state |
| [Sync Pipeline](sync.md) | The sync mode lifecycle, the block-fetch pipeline, and peer management |
| [Storage Schema](storage.md) | What `verity-db` persists, how it is keyed, which transitions commit together, and what stays in memory |
| [Key Management](key-management.md) | The crash-safe XMSS no-reuse guarantee, key material loading, and preparation scheduling |
| [Verification Tooling](model-check.md) | Which verification technique applies to which zone, classified by assurance strength |

Each document states its own status and the upstream revision it was read at. Where two
disagree, the one with the later `last_updated` is current — and the disagreement is a
defect worth reporting.
