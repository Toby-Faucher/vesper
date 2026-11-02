#!/usr/bin/env python3
"""Usage: scripts/version-only.py BASE HEAD

Exit 0 when the changes from BASE to HEAD only bump the release version (a release PR), else 1.
CI then skips the e2e and dsp-wasm jobs. Any other change, or any error, means the full pipeline.
Allowed: the [workspace.package] version in Cargo.toml, workspace member versions in Cargo.lock,
the root version in web/package.json and web/package-lock.json, and any edit to docs/versions.md.
"""
import json
import subprocess
import sys
import tomllib

ALLOWED = {"Cargo.toml", "Cargo.lock", "web/package.json", "web/package-lock.json", "docs/versions.md"}


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def without_version(path, text):
    if path == "Cargo.toml":
        d = tomllib.loads(text)
        del d["workspace"]["package"]["version"]
        return d
    if path == "Cargo.lock":
        d = tomllib.loads(text)
        for p in d["package"]:
            if "source" not in p:  # workspace members; registry and git packages have a source
                del p["version"]
        return d
    d = json.loads(text)
    del d["version"]
    if path == "web/package-lock.json":
        del d["packages"][""]["version"]
    return d


def main(base, head):
    for path in git("diff", "--name-only", "--no-renames", "-z", base, head).split("\0"):
        if not path or path == "docs/versions.md":
            continue
        if path not in ALLOWED:
            print(f"full pipeline: {path} changed")
            return 1
        # A deleted file makes git show fail, which also means the full pipeline.
        if without_version(path, git("show", f"{base}:{path}")) != without_version(path, git("show", f"{head}:{path}")):
            print(f"full pipeline: {path} changes more than the version")
            return 1
    print("version bump only: light pipeline")
    return 0


if __name__ == "__main__":
    sys.exit(main(*sys.argv[1:3]))
