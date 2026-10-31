#!/usr/bin/env bash
# Tests scripts/check-secrets.sh against a throwaway git tree. Run: scripts/test-check-secrets.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
fails=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fails=$((fails + 1)); }
git -C "$tmp" init -q
mkdir "$tmp/scripts"; cp "$here/secrets-allow.txt" "$tmp/scripts/"
# expect <pass|fail> <file content>
expect() {
  printf '%s\n' "$2" > "$tmp/x.txt"; git -C "$tmp" add x.txt
  out=$(SECRETS_ROOT=$tmp "$here/check-secrets.sh" 2>&1); rc=$?
  if { [ "$1" = fail ] && [ $rc != 0 ]; } || { [ "$1" = pass ] && [ $rc = 0 ]; }; then ok "$1: $2"; else bad "$1: $2 ($out)"; fi
}
expect fail 'VESPER_DEV_KEY=hunter2real'
expect fail 'VESPER_DEV_KEY="hunter2real"'
expect fail "VESPER_DEV_KEY='hunter2real'"
expect fail 'VESPER_DEV_KEY: hunter2real'
expect pass 'VESPER_DEV_KEY=pick-a-passphrase'
expect pass "VESPER_DEV_KEY='{{key}}'"
expect pass 'VESPER_DEV_KEY=${KEY}'
expect pass 'VESPER_DEV_KEY="${KEY}"'
expect pass 'VESPER_DEV_KEY=<your key>'
[ $fails = 0 ] && echo "all ok" || { echo "$fails failed"; exit 1; }
