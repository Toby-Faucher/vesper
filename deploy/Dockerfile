# Build from the repo root:  docker build -f deploy/Dockerfile -t vesper .
# No secrets are baked in: VESPER_DEV_KEY is supplied at run time only.

# --- 1. denoiser wasm (scripts/build-dsp.sh) ---------------------------------------------------
FROM rust:1-bookworm AS dsp
RUN rustup target add wasm32-unknown-unknown
WORKDIR /src
COPY . .
# The wasm-bindgen CLI must equal the crate version in Cargo.lock exactly (build-dsp.sh checks too).
RUN cargo install wasm-bindgen-cli --locked \
      --version "$(awk '/^name = "wasm-bindgen"$/ {getline; gsub(/[^0-9.]/, "", $3); print $3; exit}' Cargo.lock)"
# Fetches the DeepFilterNet3 model (sha256-checked) if missing, then builds into web/public/dsp/.
RUN scripts/build-dsp.sh

# --- 2. web app ---------------------------------------------------------------------------------
FROM node:22-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
COPY --from=dsp /src/web/public/dsp ./public/dsp
RUN npm run build

# --- 3. server ----------------------------------------------------------------------------------
FROM rust:1-bookworm AS server
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p vesper-server

# --- 4. runtime ---------------------------------------------------------------------------------
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --no-create-home --uid 10001 vesper
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
