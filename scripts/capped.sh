#!/usr/bin/env bash
# shellcheck shell=bash
# Runs builds under a memory cap so a runaway cargo or npm is OOM-killed inside its own cgroup
# instead of hanging the dev box (a heavy build has stalled it before). No root: it uses a
# transient systemd USER scope (systemd-run --user --scope), plus nice 19 and idle IO.
#   run:     scripts/capped.sh cargo build --release     (checks the cap, then runs the command)
#   sourced: . scripts/capped.sh; cap_check || exit 1; capped cargo test
# VESPER_NO_CAP=1        runs plain and quietly (CI runners).
# VESPER_CAP_MIB=N       hard ceiling in MiB (default 8192), swap off inside the scope.
# No user systemd here (ssh without linger, containers): cap_check warns and runs nice/ionice only.
# A scope that starts but does not enforce the ceiling makes cap_check fail: refuse, do not pretend.
if [ -n "${VESPER_NO_CAP:-}" ]; then
  capped() { "$@"; }
  cap_check() { :; }
else
  CAP_MAX_MIB=${VESPER_CAP_MIB:-8192}
  CAP_PLAIN=
  capped() {
    if [ -n "$CAP_PLAIN" ]; then
      nice -n 19 ionice -c3 "$@"
    else
      # No MemoryHigh: with swap off, a scope over MemoryHigh stalls in reclaim for minutes instead
      # of dying (seen on the dev box with a 256 MiB test), so only the hard ceiling is set.
      systemd-run --user --scope -q -p MemoryMax="${CAP_MAX_MIB}M" -p MemorySwapMax=0 -p CPUWeight=20 \
        nice -n 19 ionice -c3 "$@"
    fi
  }

  # Succeeds if the kernel enforces MemoryMax inside the scope, or (with a warning) if there is no
  # user systemd at all. Fails if the scope runs but the ceiling reads back wrong.
  cap_check() {
    local v
    case $CAP_MAX_MIB in '' | *[!0-9]*) echo "capped.sh: VESPER_CAP_MIB must be a whole number of MiB" >&2; return 1 ;; esac
    if ! command -v systemd-run >/dev/null || ! systemd-run --user --scope -q true 2>/dev/null; then
      echo "capped.sh: WARNING: no systemd user scope here, so this runs WITHOUT a memory cap (nice/ionice only). VESPER_NO_CAP=1 silences this." >&2
      CAP_PLAIN=1
      return 0
    fi
    # shellcheck disable=SC2016 # expanded by the sh inside the scope
    v=$(capped sh -c 'cat /sys/fs/cgroup$(cut -d: -f3 /proc/self/cgroup)/memory.max') \
      || { echo "capped.sh: the capped scope failed; refusing to run uncapped (VESPER_NO_CAP=1 to skip the cap)" >&2; return 1; }
    [ "$v" = $((CAP_MAX_MIB << 20)) ] \
      || { echo "capped.sh: memory cap not enforced (memory.max='$v'); refusing to run" >&2; return 1; }
  }
fi

# Run mode: executed, not sourced.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  set -euo pipefail
  [ $# -gt 0 ] || { echo "usage: scripts/capped.sh COMMAND [ARGS...]" >&2; exit 2; }
  export PATH=$HOME/.cargo/bin:$PATH
  cap_check || exit 1
  capped "$@"
fi
