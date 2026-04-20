# syntax=docker/dockerfile:1.6
FROM rust:1.82-slim-bookworm AS builder

RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache des dépendances
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
 && cargo build --release \
 && rm -rf src target/release/rust_tor_snapshotter* target/release/deps/rust_tor_snapshotter*

# Build réel
COPY . .
ENV SQLX_OFFLINE=true
RUN cargo build --release

# --- runtime ---
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini wget \
 && rm -rf /var/lib/apt/lists/* \
 && useradd -r -u 10001 -m -d /data snapper

COPY --from=builder /build/target/release/rust_tor_snapshotter /usr/local/bin/rust_tor_snapshotter

USER snapper
WORKDIR /data
VOLUME ["/data"]

# Port d'écoute — surchargeable à l'exécution via `-e PORT=9090` ou via
# compose (`environment: PORT: ...`). `BIND_ADDR` reste prioritaire si
# positionné. EXPOSE est une info ; le mapping hôte se fait côté compose.
ARG PORT=8080
ENV PORT=${PORT}
EXPOSE ${PORT}

ENV DATA_DIR=/data \
    CACHE_DIR=/data/snapshots \
    GOOGLE_SERVICE_ACCOUNT=/data/service_account.json \
    RUST_LOG=info

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD wget -qO- "http://127.0.0.1:${PORT}/api/health" >/dev/null 2>&1 || exit 1

ENTRYPOINT ["/usr/bin/tini","--"]
CMD ["/usr/local/bin/rust_tor_snapshotter"]
