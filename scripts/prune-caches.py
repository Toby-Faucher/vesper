#!/usr/bin/env python3
"""Usage: scripts/prune-caches.py [--apply] [LIST.json]

Keeps only the newest Actions cache of each kind on main, so superseded copies do not fill the repo's 10 GB and push
out the caches jobs still use. A kind is a key with its hash-like parts (8 or more hex characters) removed, for example
`v0-rust-e2e-Linux-x64` or `libvpx-v1-Windows`. Reads `gh cache list --ref refs/heads/main --json id,key,createdAt`
(or LIST.json), prints what it would delete, and deletes it with --apply.
"""
import json
import re
import subprocess
import sys

HASH = re.compile(r"^[0-9a-f]{8,}$")


def kind(key):
    # Drop every hash part, keep the rest: `v0-rust-vendor-<hash>-Linux-x64-...` and the same key for Windows must stay
    # two kinds (only the first hash differs before the OS name).
    return "-".join(p for p in key.split("-") if not HASH.match(p))


def stale(caches):
    newest = {}
    for c in caches:
        k = kind(c["key"])
        if k not in newest or c["createdAt"] > newest[k]["createdAt"]:
            newest[k] = c
    keep = {c["id"] for c in newest.values()}
    return sorted((c for c in caches if c["id"] not in keep), key=lambda c: c["key"])


def main(argv):
    apply = "--apply" in argv
    files = [a for a in argv[1:] if a != "--apply"]
    if files:
        caches = json.load(open(files[0]))
    else:
        out = subprocess.run(["gh", "cache", "list", "--ref", "refs/heads/main", "--limit", "200",
                              "--json", "id,key,createdAt"], check=True, capture_output=True, text=True).stdout
        caches = json.loads(out)
    for c in stale(caches):
        print(f"{'delete' if apply else 'would delete'} {c['key']} ({c['createdAt']})")
        if apply:
            subprocess.run(["gh", "cache", "delete", str(c["id"])], check=True)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
