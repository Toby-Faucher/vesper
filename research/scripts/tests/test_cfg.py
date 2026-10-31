import sys
import tomllib
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import cfg  # noqa: E402

CFG = tomllib.loads((Path(__file__).resolve().parents[2] / "config.toml").read_text())
BASE_OPUS = CFG["eval_baseline"]["opus"]


class EvalOpts(unittest.TestCase):
    def test_no_eval_toml_changes_nothing(self):
        base, cand = cfg.eval_opts(CFG, None)
        self.assertEqual(base, {"model": CFG["paths"]["model"], "opus": ""})
        self.assertEqual(cand["model"], CFG["paths"]["model"])
        self.assertEqual(cand["opus"], "")
        self.assertFalse(cand["allow_more_bitrate"])

    def test_model_override_is_candidate_only(self):
        m = "research/models/DeepFilterNet3_ll_onnx.tar.gz"
        base, cand = cfg.eval_opts(CFG, {"model": m})
        self.assertEqual(cand["model"], m)
        self.assertEqual(base["model"], CFG["paths"]["model"])
        self.assertEqual(base["opus"] + cand["opus"], "")

    def test_bad_model_paths(self):
        for m in ("/etc/passwd", "research/models/../../x.tar.gz", "crates/x.tar.gz", 3):
            with self.assertRaises(ValueError):
                cfg.eval_opts(CFG, {"model": m})

    def test_opus_puts_both_sides_through_opus(self):
        base, cand = cfg.eval_opts(CFG, {"opus": "bitrate=48000,dtx=0"})
        self.assertEqual(base["opus"], BASE_OPUS)
        got = cfg.parse_opus(cand["opus"])
        self.assertEqual(got["bitrate"], "48000")
        self.assertEqual(got["dtx"], "0")
        # Missing keys come from the baseline, not from audiolab's defaults.
        for k in ("loss", "seed", "fec", "complexity"):
            self.assertEqual(got[k], cfg.parse_opus(BASE_OPUS)[k])

    def test_channel_must_match_baseline(self):
        b = cfg.parse_opus(BASE_OPUS)
        cfg.eval_opts(CFG, {"opus": f"loss={b['loss']},seed={b['seed']}"})
        for bad in ("loss=0.0", "seed=2", "burst=3"):
            with self.assertRaises(ValueError):
                cfg.eval_opts(CFG, {"opus": bad})

    def test_bad_opus_strings(self):
        for bad in ("bitrate", "foo=1", "bitrate=48000 --x", "bitrate=$(id)"):
            with self.assertRaises(ValueError):
                cfg.eval_opts(CFG, {"opus": bad})

    def test_unknown_keys_and_flag_type(self):
        with self.assertRaises(ValueError):
            cfg.eval_opts(CFG, {"modle": "x"})
        with self.assertRaises(ValueError):
            cfg.eval_opts(CFG, {"allow_more_bitrate": "yes"})
        _, cand = cfg.eval_opts(CFG, {"opus": "bitrate=96000", "allow_more_bitrate": True})
        self.assertTrue(cand["allow_more_bitrate"])

    def test_opus_needs_a_baseline_setting(self):
        with self.assertRaises(ValueError):
            cfg.eval_opts({"paths": CFG["paths"]}, {"opus": "bitrate=48000"})


if __name__ == "__main__":
    unittest.main()
