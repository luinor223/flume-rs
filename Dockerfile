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
FROM rust:1.96.0-slim-bookworm@sha256:b5f842fac1e3b4ff718a652a8e0173b62d9403ec826ef4998880b9347db30684 AS builder

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
FROM gcr.io/distroless/cc-debian13:nonroot@sha256:4cf9e68a5cbd8c9623480b41d5ed6052f028c44cc29f91b21590613ab8bec824

COPY --from=builder /src/target/release/flume-server /usr/local/bin/jobmanager
COPY --from=builder /src/target/release/flume-tm /usr/local/bin/taskmanager
COPY --from=builder /src/target/release/flume-cli /usr/local/bin/cli

EXPOSE 8080 50051 50052

ENTRYPOINT []
