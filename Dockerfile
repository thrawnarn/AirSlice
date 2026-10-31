# syntax=docker/dockerfile:1

# ---- frontend -----------------------------------------------------------
FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

# ---- backend ------------------------------------------------------------
FROM rust:1-bookworm AS server
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY backend/Cargo.toml backend/Cargo.lock ./
# Cache dependencies separately from the source.
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src
COPY backend/src ./src
RUN touch src/main.rs && cargo build --release && strip target/release/airslice

# ---- OrcaSlicer ---------------------------------------------------------
FROM ubuntu:24.04 AS orca
ARG ORCA_VERSION=2.4.2
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /opt
RUN set -eux; \
    # OrcaSlicer publishes aarch64 AppImages itself since 2.4.
    if [ "$TARGETARCH" = "arm64" ]; then ARCH="aarch64_"; else ARCH=""; fi; \
    URL="https://github.com/OrcaSlicer/OrcaSlicer/releases/download/v${ORCA_VERSION}/OrcaSlicer_Linux_AppImage_Ubuntu2404_${ARCH}V${ORCA_VERSION}.AppImage"; \
    curl -fL -o orca.AppImage "$URL"; \
    chmod +x orca.AppImage; \
    ./orca.AppImage --appimage-extract >/dev/null; \
    mv squashfs-root orcaslicer; \
    rm orca.AppImage

# ---- runtime ------------------------------------------------------------
FROM ubuntu:24.04
ARG ORCA_VERSION=2.4.2
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl tini jq \
      libgl1 libgl1-mesa-dri libegl1 libopengl0 libglu1-mesa \
      libsm6 libice6 libgtk-3-0 libwebkit2gtk-4.1-0 \
      libgstreamer1.0-0 libgstreamer-plugins-base1.0-0 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --create-home --shell /usr/sbin/nologin airslice || usermod -l airslice -d /home/airslice -m ubuntu \
    && mkdir -p /data && chown 1000:1000 /data

COPY --from=orca /opt/orcaslicer /opt/orcaslicer
COPY --from=server /src/target/release/airslice /usr/local/bin/airslice
COPY --from=web /web/dist /app/web
COPY docker/entrypoint.sh /usr/local/bin/airslice-entrypoint
COPY airslice/run.sh /usr/local/bin/airslice-addon
RUN chmod 0755 /usr/local/bin/airslice-entrypoint /usr/local/bin/airslice-addon

ENV BIND=0.0.0.0:8080 \
    DATA_DIR=/data \
    STATIC_DIR=/app/web \
    ORCA_SLICER_PATH=/opt/orcaslicer/AppRun \
    ORCA_PROFILES_DIR=/opt/orcaslicer/resources/profiles \
    ORCA_VERSION=${ORCA_VERSION} \
    RUST_LOG=info

# Starts as root only to pick the mode: the entrypoint drops to user 1000
# outside Home Assistant (see docker/entrypoint.sh).
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD curl -fsS "http://127.0.0.1:8080${BASE_PATH}/api/health" || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/airslice-entrypoint"]
CMD []
