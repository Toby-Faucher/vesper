#!/usr/bin/env bash
# Shared helpers for attempt.sh and gate.sh. Source it; do not execute it.
set -euo pipefail

RESEARCH_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(git -C "$RESEARCH_DIR" rev-parse --show-toplevel)"
SCRIPTS="$RESEARCH_DIR/scripts"
EXP_DIR="$RESEARCH_DIR/experiments"
WORKTREES="$REPO_ROOT/.worktrees"
ID_RE='^E[0-9]*(-[a-z0-9]+)+$|^E[0-9]+$'

die() { echo "error: $*" >&2; exit 1; }

cfg() { python3 "$SCRIPTS/cfg.py" get "$1"; }

# heavy CMD...: run under the controller's memory-capped scope (never the candidate's capped.sh).
heavy() { VESPER_CAP_MIB=$(cfg machine.memory_cap_mib) "$REPO_ROOT/scripts/capped.sh" "$@"; }

# bad_paths ID BASE HEAD: print changed paths that are not allowed for experiment ID, one per line.
# Allowed: under an `allowed` prefix and under no `protected` prefix. Fails if git cannot diff,
# so callers fail closed. -z and quotePath=false: non-ASCII paths are neither quoted nor split.
bad_paths() {
  local id=$1 tmpf p q ok
  local -a allowed protected
  mapfile -t allowed < <(cfg paths_policy.allowed | sed "s|{id}|$id|g")
  mapfile -t protected < <(cfg paths_policy.protected)
  tmpf=$(mktemp) || return 1
  if ! git -C "$REPO_ROOT" -c core.quotePath=false diff --no-renames --name-only -z "$2" "$3" >"$tmpf"; then
    rm -f "$tmpf"; return 1
  fi
  while IFS= read -r -d '' p; do
    ok=0
    for q in "${allowed[@]}"; do if [[ $p == "$q"* ]]; then ok=1; fi; done
    for q in "${protected[@]}"; do if [[ $p == "$q"* ]]; then ok=0; fi; done
    if (( ! ok )); then printf '%s\n' "$p"; fi
  done <"$tmpf"
  rm -f "$tmpf"
}

# weakened_tests BASE HEAD: print a line if the diff removes #[test] functions or asserts from
# vesper-dsp, or touches the golden hash that pins the shipped default output.
weakened_tests() {
  local d t_rm t_add a_rm a_add
  d=$(git -C "$REPO_ROOT" diff --no-renames -U0 "$1" "$2" -- crates/vesper-dsp/src) || return 1
  t_rm=$(grep -c '^-[[:space:]]*#\[test\]' <<<"$d" || true)
  t_add=$(grep -c '^+[[:space:]]*#\[test\]' <<<"$d" || true)
  a_rm=$(grep -cE '^-[[:space:]]*(debug_)?assert(_eq|_ne)?!' <<<"$d" || true)
  a_add=$(grep -cE '^\+[[:space:]]*(debug_)?assert(_eq|_ne)?!' <<<"$d" || true)
  if (( t_rm > t_add )); then echo "removes $((t_rm - t_add)) #[test] function(s)"; fi
  if (( a_rm > a_add + 3 )); then echo "removes $((a_rm - a_add)) assert lines"; fi
  if grep -qE '^[-+].*GOLDEN_HASH' <<<"$d"; then echo "changes GOLDEN_HASH (the default must stay bit exact)"; fi
}
