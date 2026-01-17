# =============================================================================
# SabiDB Server Dockerfile
# Multi-stage build for optimized production image
# =============================================================================

# Build stage
FROM rust:1.78-bookworm AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y \
    build-essential \
    python3 \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests first (for better caching)
COPY Cargo.toml Cargo.lock ./

# Copy all crate manifests
COPY crates/sabi-core/Cargo.toml crates/sabi-core/
COPY crates/sabi-storage/Cargo.toml crates/sabi-storage/
COPY crates/sabi-sql/Cargo.toml crates/sabi-sql/
COPY crates/sabi-runtime/Cargo.toml crates/sabi-runtime/
COPY crates/sabi-server/Cargo.toml crates/sabi-server/

# Create stub lib.rs files for dependency caching
RUN mkdir -p crates/sabi-core/src && echo "pub fn stub() {}" > crates/sabi-core/src/lib.rs
RUN mkdir -p crates/sabi-storage/src && echo "pub fn stub() {}" > crates/sabi-storage/src/lib.rs
RUN mkdir -p crates/sabi-sql/src && echo "pub fn stub() {}" > crates/sabi-sql/src/lib.rs
RUN mkdir -p crates/sabi-runtime/src && echo "pub fn stub() {}" > crates/sabi-runtime/src/lib.rs
RUN mkdir -p crates/sabi-server/src && echo "fn main() {}" > crates/sabi-server/src/main.rs
RUN mkdir -p crates/sabi-server/src/bin && echo "fn main() {}" > crates/sabi-server/src/bin/server.rs

# Build dependencies only (cached layer)
RUN cargo build --release --package sabi-server 2>/dev/null || true

# Copy actual source code
COPY crates/sabi-core/src crates/sabi-core/src
COPY crates/sabi-storage/src crates/sabi-storage/src
COPY crates/sabi-sql/src crates/sabi-sql/src
COPY crates/sabi-runtime/src crates/sabi-runtime/src
COPY crates/sabi-server/src crates/sabi-server/src

# Touch files to invalidate cache and rebuild with actual code
RUN touch crates/sabi-core/src/lib.rs \
    crates/sabi-storage/src/lib.rs \
    crates/sabi-sql/src/lib.rs \
    crates/sabi-runtime/src/lib.rs \
    crates/sabi-server/src/bin/server.rs

# Build the release binary
RUN cargo build --release --package sabi-server

# =============================================================================
# Runtime stage - minimal image
# =============================================================================
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user for security
RUN useradd -m -u 1000 -s /bin/bash sabi

# Create data directory
RUN mkdir -p /app/data && chown -R sabi:sabi /app

# Copy the built binary
COPY --from=builder /app/target/release/sabi-server /app/sabi-server

# Switch to non-root user
USER sabi

# Expose the server port
EXPOSE 8080

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Default environment variables
ENV SABI_DATA_FILE=/app/data/sabidb.data
ENV SABI_WAL_FILE=/app/data/sabidb.wal
ENV SABI_ADDR=0.0.0.0:8080
ENV RUST_LOG=info

# Volume for persistent data
VOLUME ["/app/data"]

# Run the server
ENTRYPOINT ["/app/sabi-server"]
CMD ["--data-file", "/app/data/sabidb.data", "--wal-file", "/app/data/sabidb.wal", "--addr", "0.0.0.0:8080"]
