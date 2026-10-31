#!/usr/bin/env bash
# The protected-path and test-weakening checks, on a throwaway repo. Run: bash research/scripts/tests/test_paths.sh
set -euo pipefail
src="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/research/scripts" "$tmp/crates/vesper-dsp/src"
cp "$src/config.toml" "$tmp/research/"
cp "$src/scripts/lib.sh" "$src/scripts/cfg.py" "$tmp/research/scripts/"
cd "$tmp"
git init -q && git config user.email t@example.invalid && git config user.name t
printf '#[test]\nfn a() { assert!(true); }\nconst GOLDEN_HASH: u64 = 1;\n' > crates/vesper-dsp/src/lib.rs
git add -A && git commit -qm base
base=$(git rev-parse HEAD)
# shellcheck source=../lib.sh
source research/scripts/lib.sh
fail=0
check() { # name want got
  if [[ $2 == "$3" ]]; then echo "ok   $1"; else echo "FAIL $1: want '$2' got '$3'"; fail=1; fi
}

mkdir -p research/experiments/E1-a research/experiments/E2-b
echo x > crates/vesper-dsp/src/new.rs
echo x > research/experiments/E1-a/pipeline.toml
git add -A && git commit -qm ok
check "allowed paths" "" "$(bad_paths E1-a "$base" HEAD)"

echo x > research/experiments/E2-b/x
echo x > Cargo.toml
echo x > research/scripts/evil.sh
mkdir -p "crates/vesper-dsp/src/é" && echo x > "crates/vesper-dsp/src/é/ok.rs"
git add -A && git commit -qm bad
check "protected paths" "Cargo.toml research/experiments/E2-b/x research/scripts/evil.sh" \
  "$(bad_paths E1-a "$base" HEAD | sort | tr '\n' ' ' | sed 's/ $//')"

git reset -q --hard "$base"
printf 'fn a() {}\nconst GOLDEN_HASH: u64 = 2;\n' > crates/vesper-dsp/src/lib.rs
git commit -qam weak
got=$(weakened_tests "$base" HEAD)
check "removed test flagged" 1 "$(grep -c 'test' <<<"$got")"
check "golden hash flagged" 1 "$(grep -c GOLDEN_HASH <<<"$got")"
exit $fail
