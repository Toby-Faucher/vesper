#!/usr/bin/env bash
# Fetch Microsoft's DNSMOS P.835 model into research/models/ (git-ignored) and verify sha256.
# dnsmos_local.py is the reference for our Rust port (crates/vesper-audiolab/src/dnsmos.rs). It is
# fetched for reading only and never committed.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/research/models"
BASE=https://raw.githubusercontent.com/microsoft/DNS-Challenge/master/DNSMOS
FILES="
269fbebdb513aa23cddfbb593542ecc540284a91849ac50516870e1ac78f6edd DNSMOS/sig_bak_ovr.onnx sig_bak_ovr.onnx
1ab566afe006daab32ac7073296a5d0ef99f8b82f91c7266f3ccf26113d7a28b dnsmos_local.py dnsmos_local.py
"
mkdir -p "$OUT"
echo "$FILES" | while read -r sha path name; do
  [ -n "$name" ] || continue
  dst="$OUT/$name"
  if [ ! -f "$dst" ] || [ "$(sha256sum "$dst" | cut -d' ' -f1)" != "$sha" ]; then
    tmp=$(mktemp "$OUT/.download.XXXXXX")
    curl -fsSL -o "$tmp" "$BASE/$path" || { rm -f "$tmp"; exit 1; }
    got=$(sha256sum "$tmp" | cut -d' ' -f1)
    [ "$got" = "$sha" ] || { rm -f "$tmp"; echo "checksum mismatch for $name: got $got, expected $sha" >&2; exit 1; }
    mv "$tmp" "$dst"
  fi
  echo "ok $name"
done
