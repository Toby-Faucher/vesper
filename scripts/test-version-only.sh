#!/usr/bin/env bash
# Checks scripts/version-only.py, the CI gate that gives a version-bump-only PR the light pipeline,
# against throwaway commits made from this repo's real manifests and lockfiles.
# Run: scripts/test-version-only.sh
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
cd "$T"
git init -q
git config user.email test@example.invalid
git config user.name test
mkdir -p web docs crates/vesper-dsp/src
for f in Cargo.toml Cargo.lock web/package.json web/package-lock.json docs/versions.md crates/vesper-dsp/src/lib.rs; do
  cp "$ROOT/$f" "$f"
done
git add -A
git commit -qm base
base=$(git rev-parse HEAD)
fails=0

# expect <want exit code> <name>: commits the working tree, runs the gate against base, then resets.
expect() {
  git add -A
  if [ "$2" != "no change" ] && git diff --cached --quiet; then echo "FAIL $2: the case changed nothing"; fails=$((fails + 1)); fi
  git commit -qm "$2" --allow-empty
  set +e
  python3 -I "$ROOT/scripts/version-only.py" "$base" HEAD >/dev/null 2>&1
  got=$?
  set -e
  if [ "$got" != "$1" ]; then echo "FAIL $2: exit $got, want $1"; fails=$((fails + 1)); else echo "ok   $2"; fi
  git reset -q --hard "$base"
}

v=$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)
bump() {
  sed -i "s/^version = \"$v\"/version = \"9.9.9\"/" Cargo.toml
  # Only workspace members: their version line follows a vesper-* name.
  sed -i "/^name = \"vesper-/{n;s/^version = \"$v\"/version = \"9.9.9\"/}" Cargo.lock
  sed -i "0,/\"version\": \"$v\"/s//\"version\": \"9.9.9\"/" web/package.json
  sed -i "0,/\"version\": \"$v\"/s//\"version\": \"9.9.9\"/; 0,/\"version\": \"$v\"/s//\"version\": \"9.9.9\"/" web/package-lock.json
  echo "- **v9.9.9**: test." >>docs/versions.md
}

expect 0 "no change"
bump
for f in Cargo.toml Cargo.lock web/package.json web/package-lock.json; do
  git diff --quiet "$f" && { echo "FAIL bump did not change $f"; exit 1; }
done
expect 0 "version bump only"
bump; echo "// x" >>crates/vesper-dsp/src/lib.rs
expect 1 "version bump plus code"
bump; echo "new.txt" >new.txt
expect 1 "version bump plus a new file"
sed -i '0,/^checksum = /s/^checksum = .*/checksum = "00"/' Cargo.lock
expect 1 "dependency checksum in Cargo.lock"
sed -i '0,/^name = "anyhow"$/{n;s/^version = .*/version = "0.0.1"/}' Cargo.lock
expect 1 "dependency version in Cargo.lock"
sed -i 's/^resolver = "2"/resolver = "3"/' Cargo.toml
expect 1 "other Cargo.toml change"
sed -i '0,/"devDependencies": {/s//"devDependencies": {\n    "left-pad": "1.3.0",/' web/package.json
expect 1 "new web dependency"
git rm -q docs/versions.md Cargo.lock
expect 1 "deleted Cargo.lock"

[ "$fails" = 0 ] || exit 1
echo "version-only gate: all cases pass"
