#!/usr/bin/env bash
# Fails if a tracked file looks like a secret or holds a private detail: private keys, cloud and
# chat tokens, a VESPER_DEV_KEY with a real value, home-directory paths, tailnet names, tracked
# .env or key files. Only tracked files, not history: before the repo goes public, scan history
# once with a real tool (gitleaks). False positives: one fixed string per line in
# scripts/secrets-allow.txt (a line containing it is ignored); prefer rewording.
set -uo pipefail
cd "${SECRETS_ROOT:-$(dirname "$0")/..}" || exit 2   # SECRETS_ROOT: for the test
bad=0

files=$(git ls-files | grep -E '(^|/)\.env($|\.)|\.(env|pem|key|p12|pfx)$|(^|/)id_(rsa|ecdsa|ed25519)' | grep -v '\.example$')
if [ -n "$files" ]; then echo "$files"; echo "ERROR: secret-looking file is tracked"; bad=1; fi

# A dev key value is anything but a placeholder: $VAR, <...>, ... or empty.
patterns='-----BEGIN [A-Z ]*PRIVATE KEY-----|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|VESPER_DEV_KEY[[:space:]]*[=:][[:space:]]*["'"'"']?[^$<.?{[:space:]"'"'"'][^[:space:]]{3,}|/home/[a-z][a-z0-9_-]*/|[A-Za-z0-9-]+\.ts\.net'
allow=scripts/secrets-allow.txt
hits=$(git ls-files -z -- ':!scripts/check-secrets.sh' ':!scripts/test-check-secrets.sh' ':!Cargo.lock' ':!web/package-lock.json' \
  | xargs -0 grep -I -n -E -e "$patterns" 2>/dev/null)
if [ -n "$hits" ] && [ -f "$allow" ]; then hits=$(grep -v -F -f "$allow" <<<"$hits"); fi
if [ -n "$hits" ]; then echo "$hits"; echo "ERROR: possible secret or private detail above"; bad=1; fi

[ $bad -eq 0 ] && echo "ok: no secret-looking content in tracked files"
exit $bad
