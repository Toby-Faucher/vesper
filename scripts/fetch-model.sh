#!/usr/bin/env bash
# Fetch the DeepFilterNet3 model (not committed) into crates/vesper-dsp/model/ and verify its sha256.
set -euo pipefail
REV=d375b2d8309e0935d165700c91da9de862a99c31
SHA256=c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616
DIR="$(cd "$(dirname "$0")/.." && pwd)/crates/vesper-dsp/model"
OUT="$DIR/DeepFilterNet3_onnx.tar.gz"
mkdir -p "$DIR"
TMP=$(mktemp "$DIR/.download.XXXXXX")
trap 'rm -f "$TMP"' EXIT
rm -f "$OUT"
curl -fsSL -o "$TMP" "https://github.com/Rikorose/DeepFilterNet/raw/$REV/models/DeepFilterNet3_onnx.tar.gz"
GOT=$(sha256sum "$TMP" | cut -d' ' -f1)
echo "sha256 $GOT"
if [ "$GOT" != "$SHA256" ]; then
  echo "checksum mismatch (expected $SHA256)" >&2
  exit 1
fi
mv "$TMP" "$OUT"
