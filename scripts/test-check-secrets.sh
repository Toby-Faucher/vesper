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
tok=$(printf '%064d' 7 | tr 0 a)
expect fail "Cookie: vesper_session=$tok"
expect fail "Set-Cookie: __Host-vesper_session=$tok; Secure"
expect pass 'Cookie: vesper_session=<token>'
expect pass "vesper_session=${tok:0:63}"
expect fail "https://vesper.example/invite#$tok"
expect pass 'https://vesper.example/invite#<code>'
[ $fails = 0 ] && echo "all ok" || { echo "$fails failed"; exit 1; }
