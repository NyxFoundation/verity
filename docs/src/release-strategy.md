---
title: Release Strategy
last_updated: 2026-10-29
tags:
  - release
  - semantic-versioning
  - formal-verification
  - aeneas
---

# Release Strategy

Verity uses [Semantic Versioning](https://semver.org/) for product releases. Protocol devnet
names describe network compatibility; they are not Verity versions. A release may therefore have
both a SemVer tag such as `v0.3.1` and a devnet-compatible container tag.

Verity is experimental while its version is below `1.0.0`. The project may make clean breaks
between devnets and does not preserve obsolete protocol shapes.

## Version increments

| Change | Version increment |
|---|---|
| Compatible bug fix, security fix, or performance improvement | Patch: `x.y.Z` |
| Compatible feature or operator-facing capability | Minor: `x.Y.0` |
| Devnet or protocol update during `0.x`, including a breaking update | Minor: `0.Y.0` |
| Breaking CLI, API, configuration, storage, or network-compatibility change after `1.0.0` | Major: `X.0.0` |
| Protocol-fork support that preserves existing network and operator-facing compatibility | Minor: `x.Y.0` |

During `0.x`, a minor release may contain breaking changes. A `0.x` patch should remain compatible
with its minor release, but a consensus or security correction may break that compatibility when
there is no safe alternative; the break must be called out prominently in the release notes. After
`1.0.0`, an intentional compatibility break requires a major release, including when required by a
consensus or security correction.

A proof update that publishes no new Verity artifact is not a product release and requires no
version increment. If new Verity artifacts are published solely to carry updated proof evidence,
they use a patch release and are identified as proof-evidence-only in the release notes.

## Development releases before 1.0

The `0.x` series follows implementation milestones and protocol devnet generations. Aeneas proof
work proceeds continuously, but proof completeness is not a release gate during this series.
Every `0.x` release must state in its release notes:

- the target protocol/devnet generation;
- the consensus-critical logic covered by Aeneas correspondence proofs;
- the consensus-critical logic that remains outside that proof coverage; and
- known operator-facing incompatibilities with the previous release.

The first stable release, `1.0.0`, requires both operational readiness and the proof gate below.
Operational readiness includes sustained cross-client interoperability, security review, verified
restart and sync behavior, an exercised upgrade procedure, and stable policies for the CLI, HTTP
API, configuration, and persisted data. Before the first release candidate, the `1.0.0` milestone
must turn these areas into measurable acceptance criteria, including the required interop duration,
review sign-offs, and upgrade test matrix.

## Stable-release proof gate

`1.0.0` and every later major release require Aeneas coverage for the complete
consensus-critical logic shipped by that release. For this policy, consensus-critical logic is
Verity-owned deterministic logic whose result directly determines block or vote production,
acceptance or rejection, state transition, fork choice, checkpoints, finality, proposer or
validator decisions, or another peer-visible consensus result. The proof manifest must inventory
these categories and justify every exclusion. Networking, storage engines, RPC serving, process
orchestration, and other Runtime Shell concerns are outside this full-coverage requirement unless
they contain such a decision.

The gate is satisfied only when all of the following are true:

1. Aeneas has translated the release's consensus-critical Rust logic into Lean.
2. The generated Lean has correspondence proofs against
   [formal-leanSpec](https://github.com/NyxFoundation/formal-leanSpec) for the release's protocol
   target.
3. A checked-in proof manifest enumerates the required modules and obligations. Those modules
   contain no `sorry`, `admit`, or equivalent proof bypass. The manifest pins the reviewed trusted
   axiom set; a new axiom requires a public waiver and does not count as proof of the obligation it
   assumes.
4. The manifest records the exact Verity candidate commit and source-tree hash, the immutable
   formal-leanSpec commit or tag, the protocol target, and the proof scope.
5. The proof repository's CI succeeds for that manifest.
6. An immutable tag in
   [verity-aeneas-proofs](https://github.com/NyxFoundation/verity-aeneas-proofs) identifies the
   proof commit. The final manifest also records the Verity release commit and exact CI run URL.

The proof tag, manifest path, and exact CI run URL must be included in the GitHub Release notes. A
moving branch or untagged proof commit is not sufficient evidence for a release. Proof tags must
never be moved or reused.

After `1.0.0`, the proof guarantee must remain current rather than being refreshed only at major
boundaries:

- a minor or patch release that changes consensus-critical logic must update the corresponding
  Aeneas translation and proof before release;
- a minor or patch release that does not change consensus-critical logic may reuse the preceding
  proof reference, with that fact recorded in the release notes; and
- every major release rechecks complete coverage of the consensus-critical scope, including logic
  unchanged from the preceding major release.

## Proof waivers

A repository owner, explicitly named in the waiver, may approve a narrowly scoped exception when
Aeneas cannot translate required code or an otherwise blocking proof cannot be completed. The
waiver change must receive public review before release. A waiver is exceptional; it does not
silently change the definition of proof coverage and expires at the next major release or an
earlier date stated in the waiver.

Every waiver must be public and included in the release notes. It must identify:

- the exact code and proof obligation excluded;
- why the Aeneas proof is unavailable;
- the consensus and operational impact;
- the alternative verification performed;
- the condition and deadline for removing the waiver;
- a tracking issue; and
- the approving repository owner.

A release with a waiver must not describe the waived scope as Aeneas-proven.

## Release flow

Feature work lands on `develop`. A release is promoted to `main` through a release pull request.
The release pull request must establish the intended version, draft release notes, protocol target,
and expected proof scope before promotion.

For a stable release, the final sequence is:

1. Freeze the release pull request's candidate commit. Run the repository's required quality,
   conformance, and interoperability checks on that exact candidate.
2. Generate and check the proofs against the candidate. Prepare the proof manifest and satisfy the
   proof gate or attach every approved public waiver before merge.
3. Merge the release pull request to `main`. The resulting release source-tree hash must equal the
   candidate tree recorded in the proof manifest; otherwise the candidate is invalid and the proof
   gate must run again.
4. Add the final Verity release commit and CI run URL to the proof manifest, rerun its metadata and
   proof checks, and create the immutable proof tag.
5. Create the immutable `vX.Y.Z` Git tag and GitHub Release from the verified Verity commit,
   recording the proof evidence.
6. Publish artifacts that identify both the SemVer release and their exact source commit.

A devnet label or mutable container label such as `latest` is a discovery channel, not immutable
release evidence. Operators and test infrastructure that need reproducibility should pin a SemVer
or commit-specific artifact.
