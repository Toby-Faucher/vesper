#!/usr/bin/env bash
# Checks that .dockerignore keeps database files and the data dirs out of the build context. No Docker
# needed: the patterns are evaluated with Docker's rules (** crosses dirs, * and ? stay inside one
# segment, a match on a parent dir ignores everything below it, the last matching line wins, ! re-includes).
# Run: scripts/test-dockerignore.sh
set -eu
cd "$(dirname "$0")/.."
exec python3 -I - <<'PY'
import re, sys

def rx(p):
    p = p.strip("/")
    out, i = "", 0
    while i < len(p):
        if p.startswith("**/", i): out += "(?:.*/)?"; i += 3
        elif p.startswith("**", i): out += ".*"; i += 2
        elif p[i] == "*": out += "[^/]*"; i += 1
        elif p[i] == "?": out += "[^/]"; i += 1
        else: out += re.escape(p[i]); i += 1
    return re.compile("^" + out + "$")

rules = []
for line in open(".dockerignore"):
    line = line.strip()
    if line and not line.startswith("#"):
        neg = line.startswith("!")
        rules.append((neg, rx(line[1:] if neg else line)))

def ignored(path):
    parts = path.split("/")
    res = False
    for n in range(1, len(parts) + 1):  # a parent dir match ignores the child
        sub = "/".join(parts[:n])
        for neg, r in rules:
            if r.match(sub): res = not neg
    return res

must_ignore = ["data", "data/vesper.db", "deploy/data", "deploy/data/vesper.db", "deploy/data/vesper.db-wal",
               "deploy/data/vesper.db-shm", "vesper.db", "x/y/a.db", "a.sqlite3", "deploy/.env", "target/x"]
must_keep = ["Cargo.toml", "deploy/compose.yaml", "deploy/Dockerfile", "crates/vesper-store/src/lib.rs", "web/package.json",
             "web/src/main.ts"]
bad = [p for p in must_ignore if not ignored(p)] + [p for p in must_keep if ignored(p)]
for p in must_ignore + must_keep:
    print(("FAIL " if p in bad else "ok   ") + p)
sys.exit(1 if bad else 0)
PY
