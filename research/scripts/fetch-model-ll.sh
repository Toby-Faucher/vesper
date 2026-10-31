#!/usr/bin/env bash
# Fetch the DeepFilterNet3 low-latency model into research/models/ (git-ignored) and verify its sha256.
# For a model-choice experiment: eval.toml `model = "research/models/DeepFilterNet3_ll_onnx.tar.gz"`.
set -euo pipefail
REV=d375b2d8309e0935d165700c91da9de862a99c31
SHA256=5998e58e8ba0e09bb76986ef97b84afa065a571ef282d4a1222f341e3251cf3a
DIR="$(cd "$(dirname "$0")/../.." && pwd)/research/models"
OUT="$DIR/DeepFilterNet3_ll_onnx.tar.gz"
mkdir -p "$DIR"
if [ -f "$OUT" ] && [ "$(sha256sum "$OUT" | cut -d' ' -f1)" = "$SHA256" ]; then echo "ok $OUT"; exit 0; fi
TMP=$(mktemp "$DIR/.download.XXXXXX")
trap 'rm -f "$TMP"' EXIT
curl -fsSL -o "$TMP" "https://github.com/Rikorose/DeepFilterNet/raw/$REV/models/DeepFilterNet3_ll_onnx.tar.gz"
GOT=$(sha256sum "$TMP" | cut -d' ' -f1)
if [ "$GOT" != "$SHA256" ]; then
  echo "checksum mismatch: got $GOT, expected $SHA256" >&2
  exit 1
fi
mv "$TMP" "$OUT"
echo "ok $OUT"
