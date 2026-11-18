#!/usr/bin/env bash
# The desktop client's checks (desktop/ is its own Cargo workspace): fmt, clippy -D warnings, tests.
# Builds go through scripts/capped.sh like scripts/check.sh. Usage: scripts/check-desktop.sh [fmt|clippy|test]...
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root/desktop"
"$root/scripts/build-libvpx.sh" # the share encoder's static libvpx; nothing to do once built
# Linking the desktop test binaries in parallel can pass the 8 GB cap on the dev machine: two jobs unless set or uncapped (CI).
[ -z "${VESPER_NO_CAP:-}" ] && export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
cap() { if [ -n "${VESPER_NO_CAP:-}" ]; then "$@"; else "$root/scripts/capped.sh" "$@"; fi; }
steps=("$@")
[ ${#steps[@]} -eq 0 ] && steps=(fmt clippy test)
for s in "${steps[@]}"; do
  echo "== desktop $s"
  case "$s" in
    fmt) cargo fmt --check ;;
    clippy) cap cargo clippy --all-targets --locked -- -D warnings ;;
    # nextest runs the test binaries side by side (cargo test ran them one after another); there are no doc tests.
    test) cap cargo nextest run --locked ;;
    *) echo "unknown step: $s" >&2; exit 2 ;;
  esac
done
echo "DESKTOP CHECKS PASSED"
