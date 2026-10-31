#!/usr/bin/env bash
# Fetch the UTMOS22 strong ONNX model (the `second_mos` judge) into research/models/ (git-ignored)
# and verify sha256. About 411 MB. Licence MIT (UTMOS22, Saruwatari and Koyama lab, University of
# Tokyo; ONNX export by huggingface.co/TigreGotico/utmos-onnx, pinned to a revision below).
# Optional: audiolab and the gate only score second_mos when this file exists.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/research/models"
URL=https://huggingface.co/TigreGotico/utmos-onnx/resolve/ff41b8f440cb12ecda18261f9ff7326d058275ce/utmos22_strong.onnx
SHA=ece7ddb0999d0f12ffe8d7586b3618b8b6fa89269b5152288e4440d686409f69
dst="$OUT/utmos22_strong.onnx"
mkdir -p "$OUT"
if [ ! -f "$dst" ] || [ "$(sha256sum "$dst" | cut -d' ' -f1)" != "$SHA" ]; then
  tmp=$(mktemp "$OUT/.download.XXXXXX")
  curl -fsSL -o "$tmp" "$URL" || { rm -f "$tmp"; exit 1; }
  got=$(sha256sum "$tmp" | cut -d' ' -f1)
  [ "$got" = "$SHA" ] || { rm -f "$tmp"; echo "checksum mismatch: got $got, expected $SHA" >&2; exit 1; }
  mv "$tmp" "$dst"
fi
echo "ok utmos22_strong.onnx"
