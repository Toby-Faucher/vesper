#!/usr/bin/env python3
"""Read research/config.toml.

  cfg.py get <section.key>   print one value (lists: one item per line)
  cfg.py opt <section.key>   the same, but print nothing when the key is missing
  cfg.py hash                print a short digest of the sections that decide a verdict

RESEARCH_CONFIG overrides the config path (used by the tests).
"""
import hashlib
import json
import os
import sys
import tomllib
from pathlib import Path

DEFAULT = Path(__file__).resolve().parent.parent / "config.toml"
# Sections that change what a gate run measures or decides. Paths and machine limits do not.
VERDICT_SECTIONS = ("eval", "sprt", "guards", "stack")


def load(path=None):
    path = Path(path or os.environ.get("RESEARCH_CONFIG") or DEFAULT)
    with open(path, "rb") as f:
        return tomllib.load(f)


def get(cfg, dotted):
    """Walk section.key; the key may itself contain dots or dashes (experiment ids do not)."""
    section, _, key = dotted.partition(".")
    node = cfg[section]
    for part in key.split(".") if key else []:
        node = node[part]
    return node


def digest(cfg):
    part = {k: cfg.get(k) for k in VERDICT_SECTIONS}
    blob = json.dumps(part, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(blob).hexdigest()[:12]


def main(argv):
    if len(argv) == 3 and argv[1] in ("get", "opt"):
        try:
            v = get(load(), argv[2])
        except KeyError:
            if argv[1] == "opt":
                return
            raise
        print("\n".join(map(str, v)) if isinstance(v, list) else v)
    elif len(argv) == 2 and argv[1] == "hash":
        print(digest(load()))
    else:
        sys.exit("usage: cfg.py get|opt <section.key> | hash")


if __name__ == "__main__":
    main(sys.argv)
