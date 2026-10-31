#!/usr/bin/env bash
# Build vesper-dsp (DeepFilterNet3, simd128) for the browser into web/public/dsp/ and copy the model.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
WANT=$(awk '/^name = "wasm-bindgen"$/ {getline; gsub(/[^0-9.]/, "", $3); print $3; exit}' Cargo.lock)
HAVE=$(wasm-bindgen --version | cut -d' ' -f2)
if [ "$WANT" != "$HAVE" ]; then
  echo "wasm-bindgen CLI is $HAVE but Cargo.lock has $WANT; they must match exactly." >&2
  echo "Install the CLI with: cargo install wasm-bindgen-cli --version $WANT --locked" >&2
  exit 1
fi
MODEL=crates/vesper-dsp/model/DeepFilterNet3_onnx.tar.gz
[ -f "$MODEL" ] || scripts/fetch-model.sh
OUT=web/public/dsp
RUSTFLAGS="-C target-feature=+simd128" cargo build -p vesper-dsp --lib --release \
  --target wasm32-unknown-unknown --no-default-features --features wasm
mkdir -p "$OUT"
wasm-bindgen --target web --out-dir "$OUT" target/wasm32-unknown-unknown/release/vesper_dsp.wasm
install -m 644 "$MODEL" "$OUT/model.tar.gz"
ls -la "$OUT"
