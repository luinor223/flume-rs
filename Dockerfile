# Single-image Dockerfile for flume-rs.
#
# All binaries are included in one image. The role is selected via command:
#   docker run flume jobmanager [args...]
#   docker run flume taskmanager [args...]
#   docker run flume cli [args...]
#
# Usage:
#   docker build -t flume .

# --- Build stage ---
FROM rust:1.93.1-slim-bookworm@sha256:5b9332190bb3b9ece73b810cd1f1e9f06343b294ce184bcb067f0747d7d333ea AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# 1) Copy workspace manifest and lock file.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./

# 2) Copy only Cargo.toml files and build.rs scripts to resolve the dependency tree,
#    then create minimal stub lib.rs/main.rs for each crate so cargo can compile deps.
COPY crates/ crates/
RUN find crates -name '*.rs' -not -name 'build.rs' -type f -exec sh -c 'echo "" > "$1"' _ {} \; && \
    find crates -path '*/src/main.rs' -exec sh -c 'echo "fn main() {}" > "$1"' _ {} \; && \
    find crates -path '*/bin/*.rs' -exec sh -c 'echo "fn main() {}" > "$1"' _ {} \; && \
    cargo build --release --all-features 2>/dev/null || true && \
    find crates -name '*.rs' -not -name 'build.rs' -type f -delete && \
    rm -rf target/release/.fingerprint/flume-* target/release/deps/libflume* target/release/deps/flume*

# 3) Copy real source and build. Dependencies are cached from step 2.
COPY crates/ crates/
RUN cargo build --release \
    --bin flume-server \
    --bin flume-tm \
    --bin flume-cli --features cli

# --- Runtime image ---
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:84fcd3c223b144b0cb6edc5ecc75641819842a9679a3a58fd6294bec47532bf7

COPY --from=builder /src/target/release/flume-server /usr/local/bin/jobmanager
COPY --from=builder /src/target/release/flume-tm /usr/local/bin/taskmanager
COPY --from=builder /src/target/release/flume-cli /usr/local/bin/cli

EXPOSE 8080 50051 50052

ENTRYPOINT []
