# Multi-stage Dockerfile for flume-rs binaries.
#
# Build all binaries in one stage, then copy only the needed binary
# into minimal distroless runtime images.
#
# Usage:
#   docker build --target flume-server -t flume-server .
#   docker build --target flume-tm -t flume-tm .
#   docker build --target flume-cli -t flume-cli .

# --- Build stage ---
FROM rust:1.93.1-slim-bookworm@sha256:5b9332190bb3b9ece73b810cd1f1e9f06343b294ce184bcb067f0747d7d333ea AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# 1) Copy manifests and lock file for dependency caching.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/

# 2) Create stub source files so cargo can resolve and build all dependencies
#    without the real source code. This layer is cached until Cargo.toml/lock change.
RUN find crates -name '*.rs' -type f | while read f; do \
      mkdir -p "$(dirname "$f")"; \
      case "$f" in \
        */main.rs|*/server.rs|*/cli.rs|*/tm.rs) echo 'fn main() {}' > "$f" ;; \
        */build.rs) ;; \
        *) echo '' > "$f" ;; \
      esac; \
    done && \
    cargo build --release --all-features 2>/dev/null || true

# 3) Copy real source and build. Only changed source triggers recompilation;
#    dependencies are already cached from step 2.
COPY crates/ crates/
RUN cargo build --release \
    --bin flume-server \
    --bin flume-tm \
    --bin flume-cli --features cli

# --- flume-server (JobManager) ---
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:84fcd3c223b144b0cb6edc5ecc75641819842a9679a3a58fd6294bec47532bf7 AS flume-server

COPY --from=builder /src/target/release/flume-server /usr/local/bin/flume-server

EXPOSE 8080 50051

ENTRYPOINT ["flume-server"]

# --- flume-tm (TaskManager) ---
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:84fcd3c223b144b0cb6edc5ecc75641819842a9679a3a58fd6294bec47532bf7 AS flume-tm

COPY --from=builder /src/target/release/flume-tm /usr/local/bin/flume-tm

EXPOSE 50052

ENTRYPOINT ["flume-tm"]

# --- flume-cli ---
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:84fcd3c223b144b0cb6edc5ecc75641819842a9679a3a58fd6294bec47532bf7 AS flume-cli

COPY --from=builder /src/target/release/flume-cli /usr/local/bin/flume-cli

ENTRYPOINT ["flume-cli"]
