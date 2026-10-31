import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import catalog  # noqa: E402


class Catalog(unittest.TestCase):
    def test_empty_has_a_table(self):
        with tempfile.TemporaryDirectory() as t:
            text = catalog.render(Path(t))
            self.assertIn("| id | hypothesis | status |", text)

    def test_rows_grouped_by_status(self):
        with tempfile.TemporaryDirectory() as t:
            root = Path(t)
            a = root / "experiments" / "E0-lsnr"
            a.mkdir(parents=True)
            (a / "hypothesis.md").write_text("# Raise the thresholds\n")
            (a / "result.json").write_text(json.dumps({
                "status": "accepted", "reason": "ok", "n_used": 40, "cand_sha": "a" * 40,
                "rtf": {"cand": 0.0412},
                "deltas": {m: {"mean": 0.1, "ci95": [0.05, 0.15]}
                           for m in ("ovrl", "sig", "bak", "si_sdr_i")}}))
            b = root / "experiments" / "E1-gain"
            b.mkdir()
            (b / "result.json").write_text("{not json")
            (root / "experiments" / "E2-new").mkdir()
            text = catalog.render(root)
            self.assertIn("## Accepted (1)", text)
            self.assertIn("| E0-lsnr | Raise the thresholds | accepted | +0.100 [+0.050, +0.150]", text)
            self.assertIn("0.041 | 40 |  |  | exp/E0-lsnr | aaaaaaaaaaaa | ok |", text)
            self.assertIn("## Broken (1)", text)
            self.assertIn("unreadable result.json", text)
            self.assertIn("## Pending (1)", text)

    def test_model_and_opus_columns(self):
        with tempfile.TemporaryDirectory() as t:
            root = Path(t)
            a = root / "experiments" / "E9-opus48"
            a.mkdir(parents=True)
            (a / "result.json").write_text(json.dumps({
                "status": "rejected", "model": "research/models/DeepFilterNet3_ll_onnx.tar.gz",
                "opus": "bitrate=48000,loss=0.05"}))
            self.assertIn("| DeepFilterNet3_ll_onnx.tar.gz | bitrate=48000,loss=0.05 | exp/E9-opus48 |",
                          catalog.render(root))


if __name__ == "__main__":
    unittest.main()
