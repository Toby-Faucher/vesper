#!/usr/bin/env bash
# Copy check: fails when user-facing text contains a phrase from scripts/wording-banned.txt (stock
# words that make writing sound machine-made, and the em dash). There is no allowlist: reword.
# Checked: every *.md in the repo (README.md, CLAUDE.md, SECURITY.md, docs/**) and the web app's
# strings in web/src/**/*.{svelte,ts}.
# Files come from git (tracked plus untracked-not-ignored), so ignored folders are never scanned.
# Skipped: tests (*.test.ts) and generated bindings (web/src/lib/gen).
# Usage: scripts/check-wording.sh   (WORDING_ROOT and WORDING_BANNED override the root and the list, for the test)
set -u
here=$(cd "$(dirname "$0")" && pwd)
root=${WORDING_ROOT:-$here/..}
cd "$root" || exit 2
banned=${WORDING_BANNED:-$here/wording-banned.txt}
pat=$(grep -v '^[[:space:]]*\(#\|$\)' "$banned" | paste -sd'|')
[ -n "$pat" ] || { echo "no patterns in $banned" >&2; exit 2; }
# One grep over every file (one per file took 2.5 s for about 400 files). Matches are judged by output, not by
# exit code: xargs reports 123 whenever any batch had no match.
hits=$(git ls-files -z --cached --others --exclude-standard -- '*.md' 'web/src/*.svelte' 'web/src/*.ts' \
  | grep -zv -e '\.test\.ts$' -e '^web/src/lib/gen/' | sort -z | xargs -0 -r grep -nEiH -- "$pat" || true)
if [ -n "$hits" ]; then
  printf '%s\n' "$hits"
  echo "wording check failed: rewrite the lines above in plain words" >&2
  exit 1
fi
echo "wording ok"
