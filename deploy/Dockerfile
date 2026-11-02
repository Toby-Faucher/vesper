# Build from the repo root:  docker build -f deploy/Dockerfile -t vesper .
# No secrets are baked in: accounts and sessions live in the data dir, outside the image.
# Base images are pinned by digest (the multi-arch index of each tag, read from Docker Hub on 2026-11-01), so a
# rebuild of one commit uses the same bases. Bump them on purpose: docker buildx imagetools inspect <tag>.

# --- 1. denoiser wasm (scripts/build-dsp.sh) ---------------------------------------------------
FROM rust:1-bookworm@sha256:9181b56c2a21ab1d5e73cffe04fc762426d30aa7fac3c91141dd033f49a7fc8b AS dsp
WORKDIR /src
COPY . .
# rust-toolchain.toml (the pinned version + the wasm32 target) governs inside /src; install it here.
RUN rustup toolchain install
# The wasm-bindgen CLI must equal the crate version in Cargo.lock exactly (build-dsp.sh checks too).
RUN cargo install wasm-bindgen-cli --locked \
      --version "$(awk '/^name = "wasm-bindgen"$/ {getline; gsub(/[^0-9.]/, "", $3); print $3; exit}' Cargo.lock)"
# Fetches the DeepFilterNet3 model (sha256-checked) if missing, then builds into web/public/dsp/.
RUN scripts/build-dsp.sh

# --- 2. web app ---------------------------------------------------------------------------------
FROM node:22-bookworm-slim@sha256:4b150b48b4116e2a22f2c39cc27025fbf4f153e2078fd3b14d159fdb5c7279da AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
COPY --from=dsp /src/web/public/dsp ./public/dsp
RUN npm run build

# --- 3. server ----------------------------------------------------------------------------------
FROM rust:1-bookworm@sha256:9181b56c2a21ab1d5e73cffe04fc762426d30aa7fac3c91141dd033f49a7fc8b AS server
WORKDIR /src
COPY . .
# Install the pinned toolchain explicitly; do not rely on rustup auto-installing it.
RUN rustup toolchain install
# The build id in `hello` and /health (crates/vesper-server/build.rs). .git is not in the context, so
# without this arg it comes from build-id.txt as `git archive` filled it in, else it is the version.
ARG VESPER_BUILD_ID=
RUN cargo build --release --locked -p vesper-server

# --- 4. runtime ---------------------------------------------------------------------------------
FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --no-create-home --uid 10001 vesper \
 && install -d -m 0700 -o 10001 -g 10001 /data
COPY --from=server /src/target/release/vesper-server /usr/local/bin/vesper-server
COPY --from=web /web/dist /srv/vesper/web
ENV VESPER_WEB_DIR=/srv/vesper/web \
    VESPER_HTTP_BIND=0.0.0.0:8080 \
    VESPER_UDP_BIND=0.0.0.0:40000
USER vesper
EXPOSE 8080/tcp 40000/udp
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8080/health || exit 1
CMD ["vesper-server"]
