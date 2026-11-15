#!/usr/bin/env bash
# Desktop against web, picture by picture (web/e2e/parity.mjs): builds what is missing, then runs the scenes named
# (all when none). Usage: desktop/scripts/parity.sh [login shell-empty timeline ...]
# Needs sway and grim (a headless sway of its own, never the session's), Playwright's Chromium and the desktop build
# headers. Writes web/e2e/parity/<scene>-{web,desktop,side,diff}.png. node stops sway, the app and the server itself on
# exit, a crash or a signal (track() in web/e2e/lib.mjs).
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cap="$root/scripts/capped.sh"
[ -x "$root/target/release/vesper-server" ] || (cd "$root" && "$cap" cargo build --release -p vesper-server)
[ -d "$root/web/node_modules" ] || (cd "$root/web" && "$cap" npm ci)
[ -f "$root/web/dist/index.html" ] || (cd "$root/web" && "$cap" npm run build)
# A debug build: the VESPER_DEV_* hooks exist only there. cargo is quick when nothing changed.
(cd "$root/desktop" && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" "$cap" cargo build)
cd "$root/web"
exec node e2e/parity.mjs "$@"
