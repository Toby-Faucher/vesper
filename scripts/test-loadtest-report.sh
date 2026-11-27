#!/usr/bin/env bash
# Checks scripts/loadtest-report.py's verdicts and Markdown on saved results (no server runs).
# Run: scripts/test-loadtest-report.sh
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
fails=0

# result <speakers> <sharers> <p99 ms> <worst pair %> <cpu %>: one vesper-loadtest --json summary.
result() {
  cat <<EOF
{"speakers": $1, "sharers": $2, "share_mbps": 10, "seconds": 10, "pass_95": true,
 "packets_sent": 4800, "packets_expected": 14400, "packets_received": 14400, "loss_percent": 0.0,
 "worst_pair_percent": $4,
 "latency_ms": {"count": 14400, "p50": 2.1, "p95": 4.0, "p99": $3, "max": 9.0},
 "jitter_ms": {"pairs": 12, "mean": 1.0, "max": 1.5}, "server_cpu_percent": $5,
 "server_rss_mib": {"idle": 18.0, "load": 20.0, "peak": 20.0}, "video_worst_pair_percent": null}
EOF
}

# doc <file> <results...>: a results file as loadtest-report.py saves it.
doc() {
  local f=$1
  shift
  local IFS=,
  cat >"$f" <<EOF
{"meta": {"describe": "v0.0.0-test", "date": "2026-01-01 00:00", "commit": "abc", "dirty": false,
  "branch": "main", "loadtest_commit": "abc", "host": "box", "cpu": "cpu", "threads": 4,
  "mem_gib": 8, "kernel": "6.0", "loadavg": "0.1 0.1 0.1", "seconds": 10},
 "results": [$*], "errors": []}
EOF
}

# expect <want exit> <name> <file> [grep pattern]: renders the file, checks the exit and the text.
expect() {
  set +e
  python3 -I "$ROOT/scripts/loadtest-report.py" --render "$3" --out "$T/out.md" >/dev/null 2>&1
  got=$?
  set -e
  if [ "$got" != "$1" ]; then
    echo "FAIL $2: exit $got, want $1"
    fails=$((fails + 1))
  elif [ -n "${4:-}" ] && ! grep -q -- "$4" "$T/out.md"; then
    echo "FAIL $2: report lacks '$4'"
    fails=$((fails + 1))
  else
    echo "ok   $2"
  fi
}

doc "$T/pass.json" "$(result 4 0 5.0 100.0 3.0)" "$(result 4 2 6.0 100.0 15.0)"
expect 0 "all under the thresholds pass" "$T/pass.json" "Result: \*\*PASS\*\*"
expect 0 "a share scenario is named" "$T/pass.json" "4 speakers, 2 sharers x 10 Mbps"

doc "$T/slow.json" "$(result 4 0 5.0 100.0 3.0)" "$(result 12 0 15.1 100.0 3.0)"
expect 1 "p99 over 15 ms fails" "$T/slow.json" "FAIL: Forwarding latency p99"

doc "$T/lossy.json" "$(result 8 0 5.0 98.9 3.0)"
expect 1 "a pair under 99% fails" "$T/lossy.json" "FAIL: Audio delivered"

doc "$T/nocpu.json" "$(result 8 0 5.0 100.0 null)"
expect 1 "a missing CPU figure fails" "$T/nocpu.json" "FAIL: Server CPU"

doc "$T/edge.json" "$(result 8 0 15.0 99.0 50.0)"
expect 0 "values on the limits pass" "$T/edge.json" "Result: \*\*PASS\*\*"

set +e
python3 -I "$ROOT/scripts/loadtest-report.py" --sizes 4,16 >"$T/err" 2>&1
got=$?
set -e
if [ "$got" = 0 ] || ! grep -q "room cap" "$T/err"; then
  echo "FAIL a size over the room cap is refused (exit $got)"
  fails=$((fails + 1))
else
  echo "ok   a size over the room cap is refused"
fi

[ "$fails" = 0 ] || { echo "$fails failed"; exit 1; }
