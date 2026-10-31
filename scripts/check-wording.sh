#!/usr/bin/env bash
# Copy check: fails when user-facing text contains a phrase from scripts/wording-banned.txt (stock
# words that make writing sound machine-made, and the em dash). There is no allowlist: reword.
# Checked: every *.md in the repo (README.md, CLAUDE.md, SECURITY.md, docs/**) and the web app's
# strings in web/src/**/*.{svelte,ts}.
# Skipped: tests (*.test.ts), generated bindings (web/src/lib/gen), node_modules, dist, target, .git,
# .claude, .superpowers. docs/superpowers is checked too; it passes today.
# Usage: scripts/check-wording.sh   (WORDING_ROOT and WORDING_BANNED override the root and the list, for the test)
set -u
root=${WORDING_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
cd "$root" || exit 2
banned=${WORDING_BANNED:-$(dirname "$0")/wording-banned.txt}
pat=$(grep -v '^[[:space:]]*\(#\|$\)' "$banned" | paste -sd'|')
[ -n "$pat" ] || { echo "no patterns in $banned" >&2; exit 2; }
bad=0
while IFS= read -r f; do
  if grep -nEi -- "$pat" "$f" | sed "s|^|$f:|" | grep .; then bad=1; fi
done < <(find . -type f \( -name '*.md' -o -path './web/src/*' \( -name '*.svelte' -o -name '*.ts' \) \) \
  -not -path './node_modules/*' -not -path './web/node_modules/*' -not -path './web/dist/*' \
  -not -path './target/*' -not -path './.git/*' -not -path './web/src/lib/gen/*' \
  -not -path './.superpowers/*' -not -path './.claude/*' \
  -not -name '*.test.ts' | sed 's|^\./||' | sort)
if [ $bad = 1 ]; then
  echo "wording check failed: rewrite the lines above in plain words" >&2
  exit 1
fi
echo "wording ok"
