---
title: SSZ PR #132 hash_tree_root benchmark
last_updated: 2026-09-09
tags:
  - ssz
  - benchmark
  - lean
---

# SSZ PR #132 `hash_tree_root` benchmark

## Result

The PR #132 path produced the same root as libssz for every measured value. It was **7.3×–69.9× slower** overall and **67.4×–69.9× slower** on the non-trivial synthetic Block/BlockBody/State cases. The stripped node binary grew by **2,812,784 bytes (11.6%)**.

This implementation is suitable only as an experimental backend in its current form. The synthetic State case falls from 3,161 to 45 roots/s, and fork choice and block production call this capability repeatedly.

## Timing

| input | SSZ bytes | libssz median | PR #132 median | slowdown | libssz roots/s | PR #132 roots/s |
|---|---:|---:|---:|---:|---:|---:|
| synthetic/BlockHeader | 112 | 867.7 ns (816.7 ns–972.8 ns) | 36.870 µs (35.903 µs–41.261 µs) | 42.5× | 1,152,535.5 | 27,122.0 |
| synthetic/AttestationData | 128 | 582.6 ns (580.1 ns–592.8 ns) | 39.274 µs (38.767 µs–41.899 µs) | 67.4× | 1,716,549.4 | 25,462.1 |
| synthetic/BlockBody-64 | 10,820 | 81.284 µs (80.948 µs–82.748 µs) | 5.479 ms (5.427 ms–5.626 ms) | 67.4× | 12,302.5 | 182.5 |
| synthetic/Block-64 | 10,904 | 79.998 µs (79.757 µs–80.450 µs) | 5.591 ms (5.480 ms–5.776 ms) | 69.9× | 12,500.3 | 178.9 |
| synthetic/State-256v-1024h | 94,662 | 316.332 µs (312.541 µs–336.504 µs) | 21.967 ms (21.605 ms–22.704 ms) | 69.4× | 3,161.2 | 45.5 |
| fixture/AttestationData | 128 | 583.8 ns (581.1 ns–595.6 ns) | 38.823 µs (38.416 µs–39.869 µs) | 66.5× | 1,712,971.0 | 25,757.9 |
| fixture/Block | 88 | 1.847 µs (1.843 µs–1.868 µs) | 44.148 µs (43.581 µs–45.480 µs) | 23.9× | 541,385.7 | 22,651.0 |
| fixture/BlockBody | 4 | 1.180 µs (1.175 µs–1.214 µs) | 8.672 µs (8.582 µs–9.842 µs) | 7.3× | 847,199.4 | 115,310.1 |
| fixture/BlockHeader | 112 | 599.5 ns (592.1 ns–614.9 ns) | 36.566 µs (35.810 µs–39.422 µs) | 61.0× | 1,668,168.2 | 27,347.6 |
| fixture/State | 342 | 10.014 µs (9.958 µs–10.157 µs) | 253.272 µs (250.815 µs–255.330 µs) | 25.3× | 99,856.9 | 3,948.3 |

Values in parentheses are the second-lowest and second-highest per-operation times from 15 samples. Each sample was calibrated to run for at least 25 ms after three warm-up calls.

## Binary size

Measured on the release `verity` executable. The PR #132 build still contains libssz because encoding/decoding outside `hash_tree_root` remains unchanged, so this is the incremental cost of the experimental backend.

| build | unstripped | stripped |
|---|---:|---:|
| libssz | 31,777,224 bytes | 24,283,472 bytes |
| PR #132 feature | 36,202,000 bytes | 27,096,256 bytes |
| increase | 4,424,776 bytes (13.9%) | 2,812,784 bytes (11.6%) |

## Method

- Host: AMD Ryzen 9 PRO 8945HS, 8 cores/16 threads, Linux x86-64 7.0.12
- Rust: 1.97.1, release profile
- Lean: 4.33.1, release compiler
- PR source: `ethereum/ssz-specs#132` commit `463a311d61353cc83de04e1c7d71834b9f354784`
- Verity base: `0c769c280be0a8ffcd0e49f0854f0a30ebc2f31c`
- Fixture asset SHA-256: `21d9de7056b4e658031dc09e50c0e9dc1b0206089253258ce69ecff01154d4bd`
- Command:

  ```bash
  VERITY_FIXTURES=/tmp/verity-fixtures \
    cargo bench --locked -p verity-chain --features lean-ssz --bench hash_tree_root
  ```

The rolling `latest` fixture asset used for this one-time measurement no longer matches the repository's existing fixture pin (`351fb8…`). The fixture pin was not changed because this benchmark does not update Verity's conformance baseline.

## Interpretation

The measured PR path is the actual Rust integration path: Rust serializes the typed value to canonical SSZ, the C ABI copies it into a Lean `ByteArray`, and PR #132 deserializes, merkleizes, and hashes it with its unmodified pure Lean SHA-256. PR #132 does not expose Rust-layout bindings or a C ABI, so serialization/deserialization is the necessary adapter cost for this feasibility implementation.

The benchmark intentionally does not substitute host SHA-256. Consequently, it measures the proved PR implementation plus boundary conversion as requested; it does not isolate Merkle-tree construction from SHA-256 performance.
