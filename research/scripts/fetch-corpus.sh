#!/usr/bin/env bash
# Fetch the audio lab speech and noise sources into research/corpus/ (git-ignored) and verify sha256.
# LibriSpeech dev-clean-2 (CC BY 4.0): the FLAC files only. DeepFilterNet sample assets: copied from a
# local DeepFilterNet checkout (DF_ASSETS overrides the path). Safe to re-run: verified files are kept.
# Then build the mixtures: cargo run -p vesper-audiolab --release -- corpus build --out research/corpus/built --seed 1
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/research/corpus"
DF_ASSETS=${DF_ASSETS:-$HOME/Projects/friendchat-spike/DeepFilterNet/assets}
TAR_URL=https://www.openslr.org/resources/31/dev-clean-2.tar.gz
TAR_SHA=176ec501490eced2d6c1f89f4f0ddc7dfe799e649e5322f8ba49fe3ff50c8012
ASSETS="
2d885e9f45c0f9381aee09a397ecd160aff1875ffb75ebe2217d21c3511b0d5d clean_freesound_33711.wav
cc774d58170fc5b8f143345ef8ff31583feef71f23f447336fdd56c8478d8a8a noise_freesound_2530.wav
cb367b36e4e9d72d112377dd57bf354e13f0b30f8402c9e841ac47639e773497 noise_freesound_573577.wav
227db737f46bca264378e1c55c25f033ada513dbfbed928edf6783e0d13ba89d rir_sim_1001_w11.7_l2.6_h2.5_rt60_0.7919.wav
"
mkdir -p "$OUT/assets"

check() { # check SHA FILE: fails if FILE is missing or does not match
  [ -f "$2" ] && [ "$(sha256sum "$2" | cut -d' ' -f1)" = "$1" ]
}

TAR="$OUT/dev-clean-2.tar.gz"
if ! check "$TAR_SHA" "$TAR"; then
  TMP=$(mktemp "$OUT/.download.XXXXXX")
  trap 'rm -f "$TMP"' EXIT
  curl -fsSL -o "$TMP" "$TAR_URL"
  check "$TAR_SHA" "$TMP" || { echo "checksum mismatch for $TAR_URL (expected $TAR_SHA)" >&2; exit 1; }
  mv "$TMP" "$TAR"
  rm -rf "$OUT/LibriSpeech"
fi
echo "ok dev-clean-2.tar.gz"
if [ ! -f "$OUT/LibriSpeech/.extracted" ]; then
  rm -rf "$OUT/LibriSpeech"
  tar -xzf "$TAR" -C "$OUT" --wildcards 'LibriSpeech/dev-clean-2/*.flac'
  touch "$OUT/LibriSpeech/.extracted"
fi
echo "ok $(find "$OUT/LibriSpeech" -name '*.flac' | wc -l) flac files"

echo "$ASSETS" | while read -r sha name; do
  [ -n "$name" ] || continue
  dst="$OUT/assets/$name"
  check "$sha" "$dst" || cp "$DF_ASSETS/$name" "$dst"
  check "$sha" "$dst" || { echo "checksum mismatch for $name (expected $sha)" >&2; exit 1; }
  echo "ok $name"
done
