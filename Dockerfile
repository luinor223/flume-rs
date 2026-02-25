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
FROM rust:1.85-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/

# Build all release binaries.
RUN cargo build --release \
    --bin flume-server \
    --bin flume-tm \
    --bin flume-cli --features cli

# --- flume-server (JobManager) ---
FROM gcr.io/distroless/cc-debian12 AS flume-server

COPY --from=builder /src/target/release/flume-server /usr/local/bin/flume-server

EXPOSE 8080 50051

ENTRYPOINT ["flume-server"]

# --- flume-tm (TaskManager) ---
FROM gcr.io/distroless/cc-debian12 AS flume-tm

COPY --from=builder /src/target/release/flume-tm /usr/local/bin/flume-tm

EXPOSE 50052

ENTRYPOINT ["flume-tm"]

# --- flume-cli ---
FROM gcr.io/distroless/cc-debian12 AS flume-cli

COPY --from=builder /src/target/release/flume-cli /usr/local/bin/flume-cli

ENTRYPOINT ["flume-cli"]
