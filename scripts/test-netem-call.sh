#!/usr/bin/env bash
# Checks scripts/netem-call.sh without root: what it prints, the Gilbert-Elliott numbers, and that it refuses bad input.
# Never passes --apply. Run: scripts/test-netem-call.sh
set -eu
cd "$(dirname "$0")/.."
fail() { echo "FAIL: $*" >&2; exit 1; }
out=$(scripts/netem-call.sh burst5 2>/dev/null)
[ "$(echo "$out" | wc -l)" = 3 ] || fail "burst5 prints 3 commands: $out"
# 5 % loss, mean burst 3: r = 33.333 %, p = 0.05 * r / 0.95 = 1.754 %
echo "$out" | grep -qx 'tc qdisc add dev lo parent 1:4 handle 40: netem loss gemodel 1.754% 33.333% 100% 0%' || fail "gemodel: $out"
echo "$out" | grep -q 'match ip dport 40000 0xffff flowid 1:4' || fail "filter: $out"
scripts/netem-call.sh wifi --dev veth0 --port 41000 2>/dev/null | grep -q 'dev veth0 .*delay 30ms 15ms loss gemodel' || fail "wifi on veth0"
scripts/netem-call.sh wifi --port 41000 2>/dev/null | grep -q 'dport 41000 ' || fail "port"
[ "$(scripts/netem-call.sh off 2>/dev/null)" = 'tc qdisc del dev lo root' ] || fail "off"
for bad in "nosuch" "loss5 --port 4x" "loss5 --dev lo;id" ""; do
  # shellcheck disable=SC2086 # split on purpose
  if scripts/netem-call.sh $bad >/dev/null 2>&1; then fail "accepted: '$bad'"; fi
done
echo "netem-call.sh: ok"
