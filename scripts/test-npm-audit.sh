#!/usr/bin/env bash
# Checks scripts/npm-audit.py, the CI gate on npm advisories, against small `npm audit --json` reports.
# Run: scripts/test-npm-audit.sh
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
fails=0

# report <file> <advisory url>:<severity> ...: writes an npm audit --json report with those advisories.
report() {
  out=$1; shift
  python3 -I - "$out" "$@" <<'PY'
import json, sys
vulns = {}
for i, arg in enumerate(sys.argv[2:]):
    url, sev = arg.rsplit(":", 1)
    vulns[f"pkg{i}"] = {"severity": sev, "via": [{"url": url, "severity": sev, "title": "t"}]}
json.dump({"vulnerabilities": vulns}, open(sys.argv[1], "w"))
PY
}

# expect <want exit code> <name> <report file>
expect() {
  set +e
  python3 -I "$T/npm-audit.py" "$3" >/dev/null 2>&1
  got=$?
  set -e
  if [ "$got" != "$1" ]; then echo "FAIL $2: exit $got, want $1"; fails=$((fails + 1)); else echo "ok   $2"; fi
}

# A copy of the gate with one test entry in ALLOWED, so these cases do not depend on the real list.
sed 's/^ALLOWED = {/ALLOWED = {"GHSA-test-test-test": "test entry",/' "$ROOT/scripts/npm-audit.py" > "$T/npm-audit.py"
grep -q 'GHSA-test-test-test' "$T/npm-audit.py" || { echo "FAIL could not add the test entry to ALLOWED"; exit 1; }

A=https://github.com/advisories
report "$T/allowed.json" "$A/GHSA-test-test-test:high"
expect 0 "only the allowed advisory" "$T/allowed.json"
report "$T/new.json" "$A/GHSA-test-test-test:high" "$A/GHSA-aaaa-bbbb-cccc:critical"
expect 1 "a new critical advisory" "$T/new.json"
report "$T/newhigh.json" "$A/GHSA-test-test-test:high" "$A/GHSA-aaaa-bbbb-dddd:high"
expect 1 "a new high advisory" "$T/newhigh.json"
report "$T/moderate.json" "$A/GHSA-test-test-test:high" "$A/GHSA-aaaa-bbbb-eeee:moderate"
expect 0 "a moderate advisory is reported, not blocking" "$T/moderate.json"
report "$T/gone.json"
expect 1 "the allowed advisory is gone: remove it from the list" "$T/gone.json"
echo '{not json' > "$T/bad.json"
expect 2 "unreadable report" "$T/bad.json"

[ "$fails" = 0 ] || exit 1
