#!/usr/bin/env bash
# Fetch the 48 kHz speech for corpus v2 into research/corpus48/ (git-ignored) and verify sha256.
# Speech: a 40-speaker subset (20 female, 20 male, 24 utterances each, about 118 MB) of the CSTR
# VCTK Corpus 0.92 (University of Edinburgh, CC BY 4.0, https://doi.org/10.7488/ds/2645), mic2
# (Sennheiser MKH 800) FLAC, 48 kHz 16-bit. The 11 GB zip is never downloaded whole: zip_range.py
# reads its central directory and the chosen members with HTTP range requests.
# The file list and hashes are in corpus48.sha256 next to this script. Noise and RIR assets: the
# DeepFilterNet samples, copied from research/corpus/assets or a DeepFilterNet checkout (DF_ASSETS).
# Safe to re-run: verified files are kept. Then build the mixtures:
#   cargo run -p vesper-audiolab --release -- corpus build --name v2 --seed 1
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HERE="$ROOT/research/scripts"
OUT="$ROOT/research/corpus48"
DF_ASSETS=${DF_ASSETS:-$HOME/Projects/friendchat-spike/DeepFilterNet/assets}
# DSpace 7 bitstream of VCTK-Corpus-0.92.zip (10.94 GB; server ETag 8a6ba2946b36fcbef0212cad601f4bfa).
ZIP_URL=https://datashare.ed.ac.uk/server/api/core/bitstreams/535f4286-e54c-4038-838c-a02285e32cb2/content
ASSETS="
cc774d58170fc5b8f143345ef8ff31583feef71f23f447336fdd56c8478d8a8a noise_freesound_2530.wav
cb367b36e4e9d72d112377dd57bf354e13f0b30f8402c9e841ac47639e773497 noise_freesound_573577.wav
227db737f46bca264378e1c55c25f033ada513dbfbed928edf6783e0d13ba89d rir_sim_1001_w11.7_l2.6_h2.5_rt60_0.7919.wav
"
mkdir -p "$OUT/vctk" "$OUT/assets"

check() { # check SHA FILE: fails if FILE is missing or does not match
  [ -f "$2" ] && [ "$(sha256sum "$2" | cut -d' ' -f1)" = "$1" ]
}

# Zip member path of a listed file name.
member() {
  case $1 in
    *.flac) echo "wav48_silence_trimmed/${1%%_*}/$1" ;;
    *) echo "$1" ;;
  esac
}

missing=()
while read -r sha name; do
  [ -n "$name" ] || continue
  check "$sha" "$OUT/vctk/$name" || missing+=("$(member "$name")")
done <"$HERE/corpus48.sha256"
if ((${#missing[@]})); then
  echo "fetching ${#missing[@]} files from the VCTK 0.92 zip (range requests)"
  # -I: the standard library only, nothing from the current or download directory.
  python3 -I "$HERE/zip_range.py" get "$ZIP_URL" "$OUT/vctk" "${missing[@]}"
fi
bad=0
while read -r sha name; do
  [ -n "$name" ] || continue
  check "$sha" "$OUT/vctk/$name" || { echo "checksum mismatch for $name (expected $sha)" >&2; bad=1; }
done <"$HERE/corpus48.sha256"
((bad == 0)) || exit 1
echo "ok $(grep -c '\.flac$' "$HERE/corpus48.sha256") VCTK files"

echo "$ASSETS" | while read -r sha name; do
  [ -n "$name" ] || continue
  dst="$OUT/assets/$name"
  check "$sha" "$dst" || cp "$ROOT/research/corpus/assets/$name" "$dst" 2>/dev/null || cp "$DF_ASSETS/$name" "$dst"
  check "$sha" "$dst" || { echo "checksum mismatch for $name (expected $sha)" >&2; exit 1; }
  echo "ok $name"
done
