#!/usr/bin/env python3
"""Minimum detectable effect of the gate's SPRT, by simulation.

  mde.py --sd 0.05 [--n 400] [--reps 2000] [--seed 1]
  mde.py --base B.json --cand C.json [...]   use the sd of the per-item OVRL deltas of two evals

Draws per-item OVRL deltas from a normal with the given sd and a true mean gain, replays the gate's
batched SPRT (sprt.sequential, the settings in research/config.toml) on them, and prints the
share of runs that accept H1 for each gain. The MDE is the smallest gain accepted in at least 80
percent of runs. Normal deltas are an assumption: real deltas have heavier tails.
"""
import argparse
import json
import math
import random
import sys
import tomllib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import sprt  # noqa: E402

CONFIG = Path(__file__).resolve().parent.parent / "config.toml"


def power(mu, sd, n, s, reps, seed):
    """Share of `reps` simulated runs that accept H1, and the mean item count used."""
    r = random.Random(seed)
    h1, used = 0, 0
    for _ in range(reps):
        xs = [r.gauss(mu, sd) for _ in range(n)]
        res = sprt.sequential(xs, s["delta0"], s["delta1"], s["alpha"], s["beta"], s["batch"])
        h1 += res["verdict"] == "H1"
        used += res["n"]
    return h1 / reps, used / reps


def sd_of(base, cand):
    b = {i["id"]: i["ovrl"] for i in json.loads(Path(base).read_text())["items"]}
    d = [i["ovrl"] - b[i["id"]] for i in json.loads(Path(cand).read_text())["items"] if i["id"] in b]
    return math.sqrt(sprt.var(d)), len(d)


def main(argv=None):
    p = argparse.ArgumentParser()
    p.add_argument("--sd", type=float)
    p.add_argument("--base")
    p.add_argument("--cand")
    p.add_argument("--n", type=int, default=400)
    p.add_argument("--reps", type=int, default=2000)
    p.add_argument("--seed", type=int, default=1)
    a = p.parse_args(argv)
    if a.sd is None:
        if not (a.base and a.cand):
            p.error("give --sd or --base and --cand")
        a.sd, n = sd_of(a.base, a.cand)
        print(f"sd of {n} per-item OVRL deltas: {a.sd:.4f}")
    with open(CONFIG, "rb") as f:
        s = tomllib.load(f)["sprt"]
    mde = None
    for k in range(0, 41):
        mu = k * 0.0025
        pw, used = power(mu, a.sd, a.n, s, a.reps, a.seed + k)
        print(f"gain {mu:+.4f}: P(accept H1) {pw:.3f}, mean items {used:.0f}")
        if mde is None and pw >= 0.8:
            mde = mu
            break
    print(f"MDE (80 percent power, n <= {a.n}, sd {a.sd:.4f}): "
          + (f"{mde:.4f} MOS" if mde is not None else "above 0.1 MOS"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
