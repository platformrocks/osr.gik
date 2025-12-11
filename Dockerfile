# =============================================================================
# GIK CLI - Multi-stage Docker Build (Linux x86_64)
# =============================================================================
#
# This Dockerfile builds the GIK CLI binary for Linux.
#
# LAYER STRATEGY:
# 1. Base layer: Rust toolchain + system dependencies (rarely changes)
# 2. Build layer: Full source compilation
# 3. Runtime layer: Minimal image with just the binary
#
# Note: For Cargo workspaces with internal dependencies, we build all crates
# together rather than caching dependencies separately, as workspace crates
# need to be compiled together.
#
# USAGE:
#   docker build -t gik-cli:latest -f Dockerfile .
#   docker build --no-cache -t gik-cli:latest -f Dockerfile .  # Fresh build
#
# EXTRACTION:
#   container_id=$(docker create gik-cli:latest)
#   docker cp "${container_id}:/usr/local/bin/gik" ./target/gik
#   docker rm "${container_id}"
#
# =============================================================================

# -----------------------------------------------------------------------------
# Stage 1: Base - Rust toolchain and system dependencies
# -----------------------------------------------------------------------------
FROM rust:slim-bookworm AS base

# Build environment configuration
ENV CARGO_INCREMENTAL=1 \
    CARGO_NET_GIT_FETCH_WITH_CLI=true \
    CARGO_TERM_COLOR=always \
    RUST_BACKTRACE=1 \
    # Optimize for faster linking
    CARGO_PROFILE_RELEASE_LTO=thin \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1

# Install build dependencies
# - build-essential: GCC toolchain for native code
# - cmake: Required by some Rust crates (e.g., ring, aws-lc-sys)
# - protobuf-compiler: Required by lance/arrow for protobuf schemas
# - pkg-config + libssl-dev: OpenSSL for HTTPS/TLS support
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    cmake \
    protobuf-compiler \
    libprotobuf-dev \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Install nightly Rust (required for edition2024 features)
RUN rustup install nightly && rustup default nightly

WORKDIR /build

# -----------------------------------------------------------------------------
# Stage 2: Build - Compile full source
# -----------------------------------------------------------------------------
FROM base AS builder

# Git hash for version info (passed from build script)
ARG GIT_HASH=unknown
ENV GIT_HASH=${GIT_HASH}

# Copy full workspace
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binary
RUN cargo build --release -p gik-cli

# Verify binary was created
RUN test -f /build/target/release/gik && \
    echo "Binary size: $(du -h /build/target/release/gik | cut -f1)"

# -----------------------------------------------------------------------------
# Stage 3: Runtime - Minimal image with just the binary
# -----------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# Runtime dependencies
# - ca-certificates: Required for HTTPS
# - libssl3: OpenSSL 3.x runtime library (Bookworm uses OpenSSL 3)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /build/target/release/gik /usr/local/bin/gik

# Verify binary works
RUN /usr/local/bin/gik --version

# Default entrypoint
ENTRYPOINT ["gik"]
CMD ["--help"]

# =============================================================================
# BUILD METADATA
# =============================================================================
LABEL org.opencontainers.image.title="GIK CLI" \
      org.opencontainers.image.description="Guided Indexing Kernel - Local-first knowledge engine" \
      org.opencontainers.image.source="https://github.com/platformrocks/osr.gik" \
      org.opencontainers.image.vendor="Guided Engineering"
