# Dev recipes. Cargo and npm go through scripts/capped.sh (memory-capped systemd user scope, no
# root). On a machine without user systemd: VESPER_NO_CAP=1 just check
set shell := ["bash", "-euo", "pipefail", "-c"]

cap := "scripts/capped.sh"
# throwaway key for run-local; set VESPER_DEV_KEY to use your own
key := env("VESPER_DEV_KEY", "pick-a-passphrase")

default:
    @just --list

# everything CI runs except the wasm build and `npm audit` (scripts/check.sh --help lists the steps)
check:
    scripts/check.sh

# fmt, clippy, tests
fast:
    scripts/check.sh fast

test:
    scripts/check.sh test

# rewrites the code; `just check` only checks it
fmt:
    cargo fmt --all

clippy:
    scripts/check.sh clippy

# server binary (release)
build:
    {{cap}} cargo build --release --locked -p vesper-server

# denoiser wasm + model into web/public/dsp/ (fetches the model once, sha256-checked)
dsp:
    {{cap}} scripts/build-dsp.sh

# web app into web/dist
web:
    [ -d web/node_modules ] || {{cap}} npm --prefix web ci --no-audit --no-fund
    {{cap}} npm --prefix web run build

# server on localhost serving web/dist at http://127.0.0.1:8080 (the mic works on localhost). Ctrl-C stops it.
run-local: build web
    [ -f web/dist/dsp/model.tar.gz ] || { just dsp && just web; }
    VESPER_DEV_KEY='{{key}}' VESPER_HTTP_BIND=127.0.0.1:8080 VESPER_UDP_BIND=127.0.0.1:40000 \
      VESPER_WEB_DIR=web/dist RUST_LOG="${RUST_LOG:-info}" target/release/vesper-server

# docs/testing/latency.md section 1: N speakers for S seconds against a throwaway local server
loadtest speakers="12" seconds="30":
    #!/usr/bin/env bash
    set -euo pipefail
    {{cap}} cargo build --release --locked -p vesper-server -p vesper-loadtest
    VESPER_DEV_KEY=throwaway-lt VESPER_HTTP_BIND=127.0.0.1:47123 VESPER_UDP_BIND=127.0.0.1:47124 \
      target/release/vesper-server &
    srv=$!
    trap 'kill $srv 2>/dev/null' EXIT
    sleep 1
    target/release/vesper-loadtest --url ws://127.0.0.1:47123/ws --key throwaway-lt \
      --room lt --speakers {{speakers}} --seconds {{seconds}} --pid $srv

# build outputs; the fetched model stays (delete crates/vesper-dsp/model by hand to refetch)
clean:
    cargo clean
    rm -rf web/dist web/public/dsp
