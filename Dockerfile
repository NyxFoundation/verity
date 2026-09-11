# syntax=docker/dockerfile:1.7
#
# The image lean-quickstart runs Verity from (`client-cmds/verity-cmd.sh`, docker mode) and
# the one hive drives. Two stages: a Rust builder, and a minimal runtime that carries the
# binary alone.
#
# The build is `--locked` on purpose: two git dependencies (leanSig, Plonky3) are pinned only
# in Cargo.lock, and a build allowed to re-resolve would silently follow their branches
# (see CLAUDE.md). RocksDB's bindings need clang at build time; nothing at runtime does.

FROM rust:1.97-bookworm AS builder
WORKDIR /verity

RUN apt-get update \
    && apt-get install -y --no-install-recommends clang libclang-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/verity/target \
    cargo build --release --locked --bin verity \
    && cp target/release/verity /usr/local/bin/verity

FROM debian:bookworm-slim AS runtime

LABEL org.opencontainers.image.source="https://github.com/NyxFoundation/verity"
LABEL org.opencontainers.image.description="Verity, the formally verified lean consensus client"
LABEL org.opencontainers.image.licenses="MIT"

# ca-certificates: checkpoint sync fetches over HTTPS.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/verity /usr/local/bin/verity
COPY LICENSE /verity/LICENSE

# 9001/udp - libp2p QUIC
# 5052     - REST API (/lean/v0/*)
# 5054     - Prometheus metrics (/metrics)
# lean-quickstart runs the container with --network host and passes the actual ports.
EXPOSE 9001/udp 5052 5054
ENTRYPOINT ["/usr/local/bin/verity"]
