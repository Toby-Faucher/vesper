#!/usr/bin/env python3
"""Read research/config.toml.

  cfg.py get <section.key>   print one value (lists: one item per line)
  cfg.py opt <section.key>   the same, but print nothing when the key is missing
  cfg.py hash                print a short digest of the sections that decide a verdict
  cfg.py evalopts [EVAL_TOML] print five lines for gate.sh: baseline model, baseline opus,
                             candidate model, candidate opus ("" for no Opus), allow_more_bitrate (0/1)

RESEARCH_CONFIG overrides the config path (used by the tests).
"""
import hashlib
import json
import os
import re
import sys
import tomllib
from pathlib import Path

DEFAULT = Path(__file__).resolve().parent.parent / "config.toml"
# Sections that change what a gate run measures or decides. Paths and machine limits do not.
VERDICT_SECTIONS = ("eval", "eval_baseline", "sprt", "guards", "stack")


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


OPUS_KEYS = ("bitrate", "complexity", "fec", "dtx", "loss", "burst", "seed", "app")
# The network, not an encoder setting: both sides must see the same losses for an exact pairing.
CHANNEL_KEYS = ("loss", "burst", "seed")
EVAL_KEYS = ("model", "opus", "allow_more_bitrate")


def parse_opus(s):
    """`key=value,...` as audiolab --opus takes it, into an ordered dict of strings."""
    out = {}
    for kv in filter(None, s.split(",")):
        k, sep, v = kv.partition("=")
        if not sep or k not in OPUS_KEYS or not re.fullmatch(r"[A-Za-z0-9.]+", v):
            raise ValueError(f"bad opus option '{kv}'")
        out[k] = v
    return out


def eval_opts(cfg, exp=None):
    """(base, cand) eval options. exp is the experiment's parsed eval.toml, or None.
    Without an `opus` key neither side gets Opus, so older experiments and caches are unchanged."""
    eb = cfg.get("eval_baseline", {})
    model = eb.get("model") or cfg["paths"]["model"]
    base = {"model": model, "opus": ""}
    cand = {"model": model, "opus": "", "allow_more_bitrate": False}
    exp = exp or {}
    unknown = sorted(set(exp) - set(EVAL_KEYS))
    if unknown:
        raise ValueError(f"unknown key(s): {', '.join(unknown)}")
    if "model" in exp:
        m = exp["model"]
        if not isinstance(m, str) or not m.startswith("research/models/") or ".." in Path(m).parts:
            raise ValueError("model must be a path under research/models/")
        cand["model"] = m
    if "allow_more_bitrate" in exp:
        if not isinstance(exp["allow_more_bitrate"], bool):
            raise ValueError("allow_more_bitrate must be true or false")
        cand["allow_more_bitrate"] = exp["allow_more_bitrate"]
    if "opus" in exp:
        if not isinstance(exp["opus"], str) or "opus" not in eb:
            raise ValueError("opus must be a string, and [eval_baseline] needs an opus key")
        b, c = parse_opus(eb["opus"]), parse_opus(exp["opus"])
        for k in CHANNEL_KEYS:
            if c.get(k, b.get(k)) != b.get(k):
                raise ValueError(f"opus {k} must match [eval_baseline] ({b.get(k)}); leave it out")
        fmt = lambda d: ",".join(f"{k}={v}" for k, v in d.items())  # noqa: E731
        base["opus"], cand["opus"] = fmt(b), fmt(b | c)
    return base, cand


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
    elif len(argv) in (2, 3) and argv[1] == "evalopts":
        try:
            exp = load(argv[2]) if len(argv) == 3 else None
            base, cand = eval_opts(load(), exp)
        except (ValueError, OSError, tomllib.TOMLDecodeError) as e:
            sys.exit(f"eval.toml: {e}")
        print(base["model"], base["opus"], cand["model"], cand["opus"],
              int(cand["allow_more_bitrate"]), sep="\n")
    else:
        sys.exit("usage: cfg.py get|opt <section.key> | hash | evalopts [EVAL_TOML]")


if __name__ == "__main__":
    main(sys.argv)
