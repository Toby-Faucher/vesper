#!/usr/bin/env python3
"""Paired SPRT and guards for the audio lab gate. Writes result.json.

  sprt.py gate --manifest M --base B.json --cand C.json --det D.json --hypothesis H.md
               --out result.json [--meta KEY=VALUE ...]
  sprt.py broken --reason TEXT --out result.json [--meta KEY=VALUE ...]

The eval runs the whole split once. The sequential rule is then replayed on the per-item OVRL deltas
in manifest order, in batches: the verdict is the one a live sequential test would have reached, and
the deltas and guards use only the items the test consumed.
"""
import argparse
import json
import math
import re
import sys
import tomllib
from pathlib import Path

CONFIG = Path(__file__).resolve().parent.parent / "config.toml"
METRICS = ("ovrl", "sig", "bak", "si_sdr_i", "lsd", "lead_in_reduction_db", "lsd_wb", "hb_err_db", "mcd",
           "second_mos")
VAR_FLOOR = 1e-12  # all-equal deltas (a no-op candidate) give zero variance


def mean(xs):
    return sum(xs) / len(xs)


def var(xs):
    if len(xs) < 2:
        return 0.0
    m = mean(xs)
    return sum((x - m) ** 2 for x in xs) / (len(xs) - 1)


def percentile(xs, q):
    s = sorted(xs)
    pos = q * (len(s) - 1)
    lo, hi = math.floor(pos), math.ceil(pos)
    return s[lo] + (s[hi] - s[lo]) * (pos - lo)


def llr(xs, d0, d1, v):
    """Normal log-likelihood ratio of mean d1 against mean d0 at variance v.
    With d0 = 0 this is (d1 / v) * sum(x - d1 / 2)."""
    return (d1 - d0) / max(v, VAR_FLOOR) * sum(x - (d0 + d1) / 2 for x in xs)


def sequential(xs, d0, d1, alpha, beta, batch):
    """Replay the batched SPRT over xs. Variance is re-estimated from all items so far at each look."""
    upper = math.log((1 - beta) / alpha)
    lower = math.log(beta / (1 - alpha))
    n, value = 0, 0.0
    looks = list(range(batch, len(xs) + 1, batch))
    if not looks or looks[-1] != len(xs):
        looks.append(len(xs))
    for n in looks:
        head = xs[:n]
        value = llr(head, d0, d1, var(head))
        if value >= upper:
            return {"verdict": "H1", "n": n, "llr": value, "lower": lower, "upper": upper}
        if value <= lower:
            return {"verdict": "H0", "n": n, "llr": value, "lower": lower, "upper": upper}
    return {"verdict": "none", "n": n, "llr": value, "lower": lower, "upper": upper}


def ci95(xs):
    m = mean(xs)
    h = 1.96 * math.sqrt(var(xs) / len(xs)) if len(xs) > 1 else 0.0
    return {"mean": m, "ci95": [m - h, m + h]}


def guards(sig_d, sdr_d, base, cand, declared_latency, g, bak_d=None, lsd_wb_d=None):
    """Each guard: value, limit, pass. base and cand are the eval JSON objects."""
    out = {}

    def add(name, value, limit, ok):
        out[name] = {"value": value, "limit": limit, "pass": bool(ok)}

    add("sig_mean", mean(sig_d), g["sig_mean_min"], mean(sig_d) >= g["sig_mean_min"])
    p10 = percentile(sig_d, 0.10)
    add("sig_p10", p10, g["sig_p10_min"], p10 >= g["sig_p10_min"])
    # DNSMOS SIG rises when less noise is removed, so BAK must not fall (found with E2-atten24).
    if bak_d is not None:
        add("bak_mean", mean(bak_d), g["bak_mean_min"], mean(bak_d) >= g["bak_mean_min"])
    add("si_sdr_i_mean", mean(sdr_d), g["si_sdr_i_mean_min"], mean(sdr_d) >= g["si_sdr_i_mean_min"])
    # The second judge: the wideband log-mel distance to the clean reference must not grow by more
    # than the margin. Fails closed when the evals have no lsd_wb (a v1 corpus or an old binary).
    if "lsd_wb_mean_max" in g:
        v = mean(lsd_wb_d) if lsd_wb_d else None
        add("lsd_wb_mean", v, g["lsd_wb_mean_max"], v is not None and v <= g["lsd_wb_mean_max"])
    rb, rc = base.get("rtf"), cand.get("rtf")
    add("rtf", rc, g["rtf_max"], rc is not None and rc <= g["rtf_max"])
    ratio = rc / rb if rb and rc is not None else None
    add("rtf_ratio", ratio, g["rtf_ratio_max"], ratio is not None and ratio <= g["rtf_ratio_max"])
    extra = cand["latency_samples"] - base["latency_samples"]
    limit = min(g["latency_max_samples"], declared_latency)
    add("latency_samples", extra, limit, extra <= max(limit, 0))
    add("clipped_items", cand["clipped_items"], 0, cand["clipped_items"] == 0)
    return out


SCORE_KEYS = ("sig", "bak", "ovrl", "si_sdr", "si_sdr_input", "si_sdr_i", "lsd",
              "lead_in_reduction_db", "peak_dbfs", "clipped", "lsd_wb", "hb_err_db", "mcd",
              "second_mos")


def deterministic(full_items, det_items):
    """True if every score of the short run equals the full run's, bit for bit (JSON floats round-trip)."""
    if not det_items or len(det_items) > len(full_items):
        return False
    return all(a["id"] == b["id"] and all(a.get(k) == b.get(k) for k in SCORE_KEYS)
               for a, b in zip(full_items, det_items))


def declared_latency(path):
    try:
        text = Path(path).read_text(encoding="utf-8")
    except OSError:
        return 0
    m = re.search(r"^Added latency \(samples\):\s*(\d+)", text, re.M)
    return int(m.group(1)) if m else 0


def decide(manifest_ids, base, cand, det, latency_decl, cfg):
    """Return the result fields (status, reason, sprt, deltas, guards, ...)."""
    s, g = cfg["sprt"], cfg["guards"]
    bi = {it["id"]: it for it in base["items"]}
    ci = {it["id"]: it for it in cand["items"]}
    missing = [i for i in manifest_ids if i not in bi or i not in ci]
    if missing:
        return {"status": "broken", "reason": f"{len(missing)} manifest items missing from an eval (first {missing[0]})"}
    pairs = [(bi[i], ci[i]) for i in manifest_ids]
    ovrl = [c["ovrl"] - b["ovrl"] for b, c in pairs]
    seq = sequential(ovrl, s["delta0"], s["delta1"], s["alpha"], s["beta"], s["batch"])
    used = pairs[: seq["n"]]
    # Metrics an eval did not compute (null or absent) are left out of their delta list.
    d = {m: [c[m] - b[m] for b, c in used if c.get(m) is not None and b.get(m) is not None]
         for m in METRICS}
    gd = guards(d["sig"], d["si_sdr_i"], base, cand, latency_decl, g, d["bak"], d["lsd_wb"])
    det_ok = deterministic(cand["items"], det["items"])
    gd["deterministic"] = {"value": det_ok, "limit": True, "pass": det_ok}
    failed = [k for k, v in gd.items() if not v["pass"]]
    if not det_ok:
        status, reason = "broken", "not deterministic: two runs gave different scores"
    elif seq["verdict"] == "H0":
        status, reason = "rejected", "SPRT accepted H0 (no gain of delta1)"
    elif failed:
        # A failed guard rules the change out; more items would not make it acceptable.
        verdict = "accepted H1" if seq["verdict"] == "H1" else "had no verdict"
        status, reason = "rejected", f"SPRT {verdict} but guards failed: " + ", ".join(failed)
    elif seq["verdict"] == "none":
        status, reason = "inconclusive", "SPRT hit the item cap without a verdict"
    else:
        status, reason = "accepted", "SPRT accepted H1 and every guard holds"
    if failed and status != "rejected":
        reason += "; guards failed: " + ", ".join(failed)
    return {
        "status": status,
        "reason": reason,
        "n_used": seq["n"],
        "n_total": len(pairs),
        "sprt": seq | {"delta0": s["delta0"], "delta1": s["delta1"], "alpha": s["alpha"],
                       "beta": s["beta"], "batch": s["batch"]},
        "deltas": {m: ci95(v) for m, v in d.items() if v},
        # Spread of the per-item OVRL deltas over the whole split, for the minimum detectable effect.
        "ovrl_delta_sd_full": math.sqrt(var(ovrl)),
        "guards": gd,
        "rtf": {"base": base.get("rtf"), "cand": cand.get("rtf")},
        "latency_samples": {"base": base["latency_samples"], "cand": cand["latency_samples"],
                            "declared": latency_decl},
    }


def _meta(pairs):
    out = {}
    for p in pairs or []:
        k, _, v = p.partition("=")
        out[k] = v
    return out


def _write(path, obj):
    Path(path).write_text(json.dumps(obj, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main(argv=None):
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    gp = sub.add_parser("gate")
    for a in ("manifest", "base", "cand", "det", "hypothesis", "out"):
        gp.add_argument("--" + a, required=True)
    gp.add_argument("--config", default=str(CONFIG))
    gp.add_argument("--meta", action="append")
    bp = sub.add_parser("broken")
    bp.add_argument("--reason", required=True)
    bp.add_argument("--out", required=True)
    bp.add_argument("--meta", action="append")
    a = p.parse_args(argv)
    if a.cmd == "broken":
        res = {"status": "broken", "reason": a.reason}
    else:
        with open(a.config, "rb") as f:
            cfg = tomllib.load(f)
        load = lambda q: json.loads(Path(q).read_text(encoding="utf-8"))  # noqa: E731
        ids = [it["id"] for it in load(a.manifest)["items"]]
        res = decide(ids, load(a.base), load(a.cand), load(a.det), declared_latency(a.hypothesis), cfg)
    res = _meta(a.meta) | res
    _write(a.out, res)
    print(f"{res['status']}  {res['reason']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
