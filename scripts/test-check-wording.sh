#!/usr/bin/env bash
# Tests scripts/check-wording.sh against a throwaway tree. Run: scripts/test-check-wording.sh
set -u
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
fails=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fails=$((fails + 1)); }
run() { WORDING_ROOT=$tmp WORDING_BANNED=$here/wording-banned.txt "$here/check-wording.sh" 2>&1; }

git -C "$tmp" init -q
mkdir -p "$tmp/docs" "$tmp/web/src/lib/gen" "$tmp/web/node_modules/x"
printf 'Plain short sentences.\n' > "$tmp/README.md"
out=$(run) && ok "clean tree passes" || bad "clean tree passes: $out"

printf 'We leverage the thing.\n' > "$tmp/docs/a.md"
out=$(run) && bad "stock word fails" || { [[ $out == *docs/a.md:1:* ]] && ok "stock word fails and names file:line" || bad "no file:line in: $out"; }

printf 'One thing \xe2\x80\x94 another.\n' > "$tmp/docs/a.md"
out=$(run) && bad "em dash fails" || ok "em dash fails"

printf 'Plain.\n' > "$tmp/docs/a.md"
printf '<p>A robust button</p>\n' > "$tmp/web/src/App.svelte"
out=$(run) && bad "svelte string fails" || ok "svelte string fails"

rm "$tmp/web/src/App.svelte"
printf 'seamless\n' > "$tmp/web/src/lib/gen/X.ts"
printf 'seamless\n' > "$tmp/web/src/a.test.ts"
printf 'seamless\n' > "$tmp/web/node_modules/x/r.md"
printf 'node_modules/\n' > "$tmp/.gitignore"
out=$(run) && ok "gen, tests and node_modules are skipped" || bad "skips: $out"

[ $fails = 0 ] && echo "all ok" || { echo "$fails failed"; exit 1; }
