#!/usr/bin/env bash
# Checks scripts/bench.py's limits, regression rule and baseline choice on saved results (no runs).
# Run: scripts/test-bench.sh
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
fails=0

# doc <file> <name> <p99 median> <p99 min> <p99 max>: a results file with one load case.
doc() {
  cat >"$1" <<EOF
{"meta": {"name": "$2", "date": "2026-01-01 00:00", "describe": "$2", "commit": "abc", "dirty": false,
  "branch": "main", "bench_commit": "abc", "dsp_note": "is the same.", "host": "box", "cpu": "cpu",
  "threads": 4, "mem_gib": 8, "kernel": "6.0", "governor": "performance", "turbo": "off", "mhz": 3000,
  "loadavg_start": "0.1 0.1 0.1", "loadavg_end": "0.1 0.1 0.1", "rustc": "rustc", "settings": "test"},
 "summary": {
  "load-12.worst_pair": {"median": 100.0, "min": 100.0, "max": 100.0, "n": 3},
  "load-12.latency_p99": {"median": $3, "min": $4, "max": $5, "n": 3},
  "load-12.cpu": {"median": 9.0, "min": 8.5, "max": 9.5, "n": 3}},
 "errors": [], "notes": []}
EOF
}

# expect <want exit> <name> <file> <baseline file or none> [grep pattern]
expect() {
  set +e
  python3 -I "$ROOT/scripts/bench.py" --render "$3" --baseline "$4" --out "$T/out.md" >/dev/null 2>&1
  got=$?
  set -e
  if [ "$got" != "$1" ]; then
    echo "FAIL $2: exit $got, want $1"
    fails=$((fails + 1))
  elif [ -n "${5:-}" ] && ! grep -q -- "$5" "$T/out.md"; then
    echo "FAIL $2: report lacks '$5'"
    fails=$((fails + 1))
  else
    echo "ok   $2"
  fi
}

doc "$T/base.json" v1.0.0 6.0 5.5 6.5
doc "$T/same.json" v1.1.0 6.2 5.8 6.6
expect 0 "no baseline, under the limits" "$T/same.json" none "Result: \*\*PASS\*\*"
expect 0 "a small move is not a regression" "$T/same.json" "$T/base.json" "no regression against v1.0.0"

doc "$T/slow.json" v1.1.0 9.0 8.6 9.4
expect 1 "p99 up 50%, ranges apart: regression" "$T/slow.json" "$T/base.json" "REGRESSION"

doc "$T/noisy.json" v1.1.0 9.0 6.0 12.0
expect 0 "p99 up 50%, ranges overlap: not flagged" "$T/noisy.json" "$T/base.json" "no regression"

doc "$T/fast.json" v1.1.0 3.0 2.8 3.2
expect 0 "faster is better, not a regression" "$T/fast.json" "$T/base.json" "better"

doc "$T/limit.json" v1.1.0 16.0 15.5 16.5
expect 1 "p99 over 15 ms misses the limit" "$T/limit.json" none "Limit missed: load-12.latency_p99"

python3 -I - "$T/slow.json" "$T/busy.json" <<'EOF2'
import json, sys
d = json.load(open(sys.argv[1]))
d["summary"]["machine.other_cores"] = {"median": 3.0, "min": 2.0, "max": 6.0, "n": 3}
json.dump(d, open(sys.argv[2], "w"))
EOF2
expect 1 "a busy machine is named in the verdict" "$T/busy.json" "$T/base.json" "FAIL, machine busy"

# Baseline choice: the newest vX.Y.0 older than the name; patch and newer reports are skipped.
mkdir -p "$T/b"
for v in v2.2.0 v2.3.0 v2.3.2 v2.4.0 main-abc; do echo '{}' >"$T/b/$v.json"; done
got=$(python3 -I - "$ROOT/scripts/bench.py" "$T/b" <<'EOF'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("bench", sys.argv[1])
b = importlib.util.module_from_spec(spec)
spec.loader.exec_module(b)
b.BENCH_DIR = sys.argv[2]
name = lambda p: p and p.rsplit("/", 1)[1]
print(name(b.find_baseline("v2.4.0", sys.argv[2] + "/v2.4.0.json")),
      name(b.find_baseline("v2.3.5", "")), name(b.find_baseline("main-abc", "")),
      name(b.find_baseline("v2.2.0", "")))
EOF
)
if [ "$got" = "v2.3.0.json v2.3.0.json v2.4.0.json None" ]; then
  echo "ok   baseline is the previous minor release"
else
  echo "FAIL baseline choice: got '$got'"
  fails=$((fails + 1))
fi

[ "$fails" = 0 ] || { echo "$fails failed"; exit 1; }
