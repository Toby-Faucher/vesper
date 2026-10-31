import math
import tempfile
import random
import sys
import tomllib
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import sprt  # noqa: E402

CFG = tomllib.loads((Path(__file__).resolve().parents[2] / "config.toml").read_text())
S = CFG["sprt"]


def run(xs):
    return sprt.sequential(xs, S["delta0"], S["delta1"], S["alpha"], S["beta"], S["batch"])


def noisy(mu, sd, n=240, seed=1):
    r = random.Random(seed)
    return [r.gauss(mu, sd) for _ in range(n)]


class Llr(unittest.TestCase):
    def test_formula_matches_spec(self):
        xs = [0.05, -0.01, 0.03, 0.0]
        v = 0.04
        want = (0.02 / v) * sum(x - 0.01 for x in xs)
        self.assertAlmostEqual(sprt.llr(xs, 0.0, 0.02, v), want, places=12)

    def test_formula_matches_normal_likelihood(self):
        xs, v = [0.1, -0.2, 0.05], 0.09
        logpdf = lambda x, m: -((x - m) ** 2) / (2 * v)  # noqa: E731
        want = sum(logpdf(x, 0.02) - logpdf(x, 0.0) for x in xs)
        self.assertAlmostEqual(sprt.llr(xs, 0.0, 0.02, v), want, places=12)

    def test_bounds(self):
        r = run([0.0] * 20)
        self.assertAlmostEqual(r["upper"], math.log(19))
        self.assertAlmostEqual(r["lower"], -math.log(19))


class Sequential(unittest.TestCase):
    def test_clear_gain_accepts_h1_early(self):
        r = run(noisy(0.05, 0.1))
        self.assertEqual(r["verdict"], "H1")
        self.assertLess(r["n"], 240)
        self.assertEqual(r["n"] % 20, 0)

    def test_no_gain_accepts_h0(self):
        r = run(noisy(0.0, 0.05))
        self.assertEqual(r["verdict"], "H0")

    def test_tiny_noisy_effect_is_inconclusive(self):
        r = run(noisy(0.01, 0.3))
        self.assertEqual(r["verdict"], "none")
        self.assertEqual(r["n"], 240)

    def test_identical_outputs_accept_h0(self):
        r = run([0.0] * 240)
        self.assertEqual((r["verdict"], r["n"]), ("H0", 20))

    def test_loss_accepts_h0(self):
        self.assertEqual(run(noisy(-0.3, 0.2))["verdict"], "H0")

    def test_partial_last_batch_is_looked_at(self):
        self.assertEqual(run([0.0] * 25)["n"], 20)
        self.assertEqual(run(noisy(0.01, 0.3, n=25))["n"], 25)


def item(i, **kw):
    base = {k: 0.0 for k in sprt.SCORE_KEYS}
    base.update(id=f"gate-{i:04}", clipped=False, **kw)
    return base


def evaljson(items, rtf=0.05, lat=0, clipped=0):
    return {"items": items, "rtf": rtf, "latency_samples": lat, "clipped_items": clipped}


class Decide(unittest.TestCase):
    ids = [f"gate-{i:04}" for i in range(240)]

    def pair(self, ovrl_gain=0.05, sig=0.0, sdr=0.0, seed=2, **cand_kw):
        r = random.Random(seed)
        base = [item(i, ovrl=2.5, sig=3.0, si_sdr_i=0.0) for i in range(240)]
        cand = [item(i, ovrl=2.5 + r.gauss(ovrl_gain, 0.1), sig=3.0 + sig, si_sdr_i=sdr)
                for i in range(240)]
        return evaljson(base), evaljson(cand, **cand_kw), evaljson(cand[:5])

    def test_accepts_clear_gain(self):
        b, c, d = self.pair()
        res = sprt.decide(self.ids, b, c, d, 0, CFG)
        self.assertEqual(res["status"], "accepted", res["reason"])
        self.assertAlmostEqual(res["deltas"]["ovrl"]["mean"], 0.05, delta=0.04)

    def test_sig_loss_guard_rejects(self):
        b, c, d = self.pair(sig=-0.05)
        res = sprt.decide(self.ids, b, c, d, 0, CFG)
        self.assertEqual(res["status"], "rejected")
        self.assertFalse(res["guards"]["sig_mean"]["pass"])

    def test_si_sdr_guard_rejects(self):
        b, c, d = self.pair(sdr=-1.0)
        res = sprt.decide(self.ids, b, c, d, 0, CFG)
        self.assertEqual(res["status"], "rejected")
        self.assertFalse(res["guards"]["si_sdr_i_mean"]["pass"])

    def test_rtf_guards(self):
        b, c, d = self.pair(rtf=0.07)  # 1.4x the baseline
        res = sprt.decide(self.ids, b, c, d, 0, CFG)
        self.assertFalse(res["guards"]["rtf_ratio"]["pass"])
        self.assertTrue(res["guards"]["rtf"]["pass"])
        self.assertEqual(res["status"], "rejected")
        b, c, d = self.pair(rtf=0.31)
        self.assertFalse(sprt.decide(self.ids, b, c, d, 0, CFG)["guards"]["rtf"]["pass"])

    def test_latency_needs_declaration(self):
        b, c, d = self.pair(lat=120)
        self.assertEqual(sprt.decide(self.ids, b, c, d, 0, CFG)["status"], "rejected")
        self.assertEqual(sprt.decide(self.ids, b, c, d, 120, CFG)["status"], "accepted")
        b, c, d = self.pair(lat=480)
        self.assertEqual(sprt.decide(self.ids, b, c, d, 480, CFG)["status"], "rejected")

    def test_clipping_rejects(self):
        b, c, d = self.pair(clipped=1)
        self.assertEqual(sprt.decide(self.ids, b, c, d, 0, CFG)["status"], "rejected")

    def test_nondeterminism_is_broken(self):
        b, c, d = self.pair()
        d["items"][2] = dict(d["items"][2], ovrl=d["items"][2]["ovrl"] + 1e-9)
        self.assertEqual(sprt.decide(self.ids, b, c, d, 0, CFG)["status"], "broken")

    def test_missing_item_is_broken(self):
        b, c, d = self.pair()
        c["items"].pop()
        self.assertEqual(sprt.decide(self.ids, b, c, d, 0, CFG)["status"], "broken")

    def test_noop_is_rejected(self):
        b, _, _ = self.pair()
        res = sprt.decide(self.ids, b, b, evaljson(b["items"][:5]), 0, CFG)
        self.assertEqual(res["status"], "rejected")
        self.assertEqual(res["deltas"]["ovrl"]["mean"], 0.0)
        self.assertEqual(res["n_used"], 20)

    def test_declared_latency_parse(self):
        with tempfile.TemporaryDirectory() as t:
            p = Path(t) / "h.md"
            p.write_text("# x\n\nAdded latency (samples): 96\n")
            self.assertEqual(sprt.declared_latency(p), 96)
            p.write_text("# x\n")
            self.assertEqual(sprt.declared_latency(p), 0)
        self.assertEqual(sprt.declared_latency("/nonexistent/h.md"), 0)


if __name__ == "__main__":
    unittest.main()
