#!/usr/bin/env bash
# Tests scripts/capped.sh against a stub systemd-run on PATH (nothing real is capped or started):
# limits are passed, the exit code passes through, an unenforced cap is refused, no user systemd
# warns and runs plain, VESPER_NO_CAP runs plain and quiet. Run: scripts/test-capped.sh
set -u
here=$(cd "$(dirname "$0")/.." && pwd)
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
L=$t/log; : > "$L"; mkdir "$t/bin"
export PATH=$t/bin:$PATH
# The stub logs its call, fails on SR_FAIL (no user manager) or SR_FAIL_SH (the probe scope dies),
# answers the memory.max probe with MEMMAX (default: the 8192 MiB cap), else runs the command.
cat > "$t/bin/systemd-run" <<EOF
#!/bin/bash
echo "systemd-run \$*" >> "$L"
[ -z "\${SR_FAIL:-}" ] || exit 1
while [ \$# -gt 0 ]; do case \$1 in -p) shift 2 ;; -*) shift ;; *) break ;; esac; done
[ "\$1" = nice ] && shift 5
if [ "\$1" = sh ] && [[ \$3 == *memory.max* ]]; then [ -z "\${SR_FAIL_SH:-}" ] || exit 1; echo "\${MEMMAX:-8589934592}"; exit 0; fi
exec "\$@"
EOF
chmod +x "$t/bin/systemd-run"

bad=0
fail() { echo "FAIL: $*"; sed 's/^/    log: /' "$L"; bad=1; }
check() { if [ "$2" = "$3" ]; then echo "ok   $1"; else fail "$1: rc $3, want $2"; fi; }
# Sources capped.sh in a subshell, runs cap_check, then the given command.
in_shell() { (unset VESPER_NO_CAP; [ -n "${NOCAP:-}" ] && export VESPER_NO_CAP=1; . "$here/scripts/capped.sh"; cap_check || exit 1; "$@"); }

out=$(in_shell capped echo hi); check "capped runs the command" 0 $?
[ "$out" = hi ] || fail "capped output: '$out'"
grep -q -- "--user --scope -q -p MemoryMax=8192M -p MemorySwapMax=0 -p CPUWeight=20 nice -n 19 ionice -c3 echo hi" "$L" \
  || fail "capped did not pass the limits"
in_shell capped false; check "capped passes the exit code" 1 $?
in_shell true; check "cap_check passes when enforced" 0 $?
MEMMAX=max in_shell true 2>/dev/null; check "cap_check refuses an unenforced cap" 1 $?
SR_FAIL_SH=1 in_shell true 2>/dev/null; check "cap_check refuses when the scope dies" 1 $?
VESPER_CAP_MIB=256 MEMMAX=268435456 in_shell true; check "VESPER_CAP_MIB is read back" 0 $?

: > "$L"
err=$(SR_FAIL=1 in_shell capped echo plain 2>&1 >/dev/null); check "no user systemd still runs" 0 $?
case $err in *WARNING*) ;; *) fail "no user systemd did not warn: '$err'" ;; esac
[ "$(grep -c . "$L")" = 1 ] || fail "no user systemd: expected only the probe call"

"$here/scripts/capped.sh" echo run >/dev/null; check "run mode runs" 0 $?
"$here/scripts/capped.sh" sh -c 'exit 7'; check "run mode passes the exit code" 7 $?
MEMMAX=max "$here/scripts/capped.sh" echo run >/dev/null 2>&1; check "run mode refuses an unenforced cap" 1 $?
"$here/scripts/capped.sh" >/dev/null 2>&1; check "run mode without a command" 2 $?

: > "$L"
out=$(NOCAP=1 in_shell capped echo plain 2>&1); check "VESPER_NO_CAP runs plain" 0 $?
[ "$out" = plain ] || fail "VESPER_NO_CAP output (want no warning): '$out'"
[ -s "$L" ] && fail "VESPER_NO_CAP still called systemd-run"
exit $bad
