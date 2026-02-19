# ── Stage 1: Build ────────────────────────────────────────────────
FROM rust:1.85-bookworm AS builder

WORKDIR /build

# 1a — Cache dependencies by building a skeleton project first.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/mc src/ui && \
    echo 'fn main() {}' > src/main.rs && \
    echo '' > src/mc/mod.rs && \
    echo '' > src/ui/mod.rs && \
    cargo build --release 2>&1 || true && \
    rm -rf src

# 1b — Copy real source and build.
COPY src/ src/
# Touch main.rs so cargo sees it as newer than the cached stub.
RUN touch src/main.rs && \
    cargo build --release && \
    strip target/release/mission_control

# ── Stage 2: Runtime ─────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates wget && \
    rm -rf /var/lib/apt/lists/*

RUN groupadd -r mc && useradd -r -g mc -d /app mc

WORKDIR /app

COPY --from=builder /build/target/release/mission_control /app/mission_control
COPY static/ /app/static/

# SQLite data directory — bind-mount a host directory here.
RUN mkdir -p /data && chown mc:mc /data

# Host bd binary is bind-mounted at /hostbin/bd.
RUN mkdir -p /hostbin

USER mc

ENV MC_BIND_HOST=0.0.0.0 \
    MC_BIND_PORT=3000 \
    MC_SQLITE_URL=sqlite:///data/mc.db \
    MC_BD_BIN=/hostbin/bd \
    MC_POLL_MS=5000 \
    RUST_LOG=info

EXPOSE 3000

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=3 \
    CMD wget -qO- http://127.0.0.1:3000/healthz || exit 1

ENTRYPOINT ["/app/mission_control"]
