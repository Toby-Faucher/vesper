#!/usr/bin/env bash
# Checks scripts/prune-caches.py: the newest cache of each kind stays, older ones go, unrelated kinds are untouched.
# Run: scripts/test-prune-caches.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp)
trap 'rm -f "$t"' EXIT
cat >"$t" <<'JSON'
[
  {"id": 1, "key": "v0-rust-e2e-Linux-x64-a74c6f26-3bbc99b7", "createdAt": "2026-11-17T15:56:00Z"},
  {"id": 2, "key": "v0-rust-e2e-Linux-x64-a74c6f26-89d48b6f", "createdAt": "2026-11-18T04:31:00Z"},
  {"id": 3, "key": "v0-rust-test-rust-Linux-x64-a74c6f26-89d48b6f", "createdAt": "2026-11-18T04:24:00Z"},
  {"id": 4, "key": "dsp-wasm-v1-Linux-054a3d00df402d5e", "createdAt": "2026-11-18T04:24:00Z"},
  {"id": 5, "key": "dsp-wasm-v1-Linux-9999aaaabbbbcccc", "createdAt": "2026-11-16T04:24:00Z"},
  {"id": 6, "key": "v0-rust-e2e-Linux-x64-0123abcd-89d48b6f", "createdAt": "2026-11-10T00:00:00Z"},
  {"id": 7, "key": "apt-desktop-v1-Linux", "createdAt": "2026-11-18T00:00:00Z"},
  {"id": 8, "key": "v0-rust-vendor-179b517460bb4619-Linux-x64-747a68e0-6df1eb27", "createdAt": "2026-11-18T07:40:00Z"},
  {"id": 9, "key": "v0-rust-vendor-a750b8bd4c12f973-Windows_NT-x64-c1fdf023-6df1eb27", "createdAt": "2026-11-18T07:45:00Z"}
]
JSON
got=$(python3 -I "$here/prune-caches.py" "$t")
want="would delete dsp-wasm-v1-Linux-9999aaaabbbbcccc (2026-11-16T04:24:00Z)
would delete v0-rust-e2e-Linux-x64-0123abcd-89d48b6f (2026-11-10T00:00:00Z)
would delete v0-rust-e2e-Linux-x64-a74c6f26-3bbc99b7 (2026-11-17T15:56:00Z)"
if [ "$got" = "$want" ]; then echo "ok   newest per kind kept"; else printf 'FAIL\ngot:\n%s\nwant:\n%s\n' "$got" "$want"; exit 1; fi
