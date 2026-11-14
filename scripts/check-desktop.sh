#!/usr/bin/env bash
# The desktop client's checks (desktop/ is its own Cargo workspace): fmt, clippy -D warnings, tests.
# Builds go through scripts/capped.sh like scripts/check.sh. Usage: scripts/check-desktop.sh [fmt|clippy|test]...
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root/desktop"
cap() { if [ -n "${VESPER_NO_CAP:-}" ]; then "$@"; else "$root/scripts/capped.sh" "$@"; fi; }
steps=("$@")
[ ${#steps[@]} -eq 0 ] && steps=(fmt clippy test)
for s in "${steps[@]}"; do
  echo "== desktop $s"
  case "$s" in
    fmt) cargo fmt --check ;;
    clippy) cap cargo clippy --all-targets --locked -- -D warnings ;;
    test) cap cargo test --locked ;;
    *) echo "unknown step: $s" >&2; exit 2 ;;
  esac
done
echo "DESKTOP CHECKS PASSED"
