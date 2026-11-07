"""Category conversion tests; all temporary files require FLOWER_TEST_SCRATCH."""
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from scripts import build_categories as build


class ConversionTests(unittest.TestCase):
    def test_text_and_df(self):
        for line, reason in [
            ("", "blank"), ("微信", "invalid_fields"), ("微信\t-1", "invalid_df"),
            ("微信\t12污染", "invalid_df"), ("微信\t1.2", "invalid_df"),
            ("微信\t１２", "invalid_df"), ("AI\t4", "unsupported_text"),
            ("字\t4", "unsupported_text"), ("字" * 17 + "\t4", "unsupported_text"),
            ("㐀字\t4", "unsupported_text"),
        ]:
            with self.subTest(line=line):
                self.assertEqual(build.parse_thu_line(line)[1], reason)
        self.assertEqual(build.parse_thu_line("微信\t0"), (("微信", 0), None))
        self.assertEqual(build.parse_thu_line("微信\t001"), (("微信", 1), None))
        self.assertEqual(build.parse_thu_line("微信 \t 123"), (("微信", 123), None))
        self.assertEqual(build.parse_thu_line("微信 \t 12 3")[1], "invalid_df")
        self.assertTrue(build.basic_word("字" * 16))

    def test_reading_priority_and_strict_monophonic(self):
        chars = {"微": {"wei"}, "信": {"xin", "shen"}, "文": {"wen"}}
        exact = {"微信": {"wei'xin": 3}}
        self.assertEqual(build.reliable_readings("微信", exact, chars),
                         ({"wei'xin": 3}, "cedict"))
        self.assertEqual(build.reliable_readings("微信", {}, chars)[1],
                         "ambiguous_character_reading")
        self.assertEqual(build.reliable_readings("微文", {}, chars),
                         ({"wei'wen": None}, "base_monophonic"))
        self.assertEqual(build.reliable_readings("微缺", {}, chars)[1],
                         "missing_character_reading")

    def test_cedict_erhua_and_normalization(self):
        normal, erhua, stats = build.parse_cedict(
            "女兒 女儿 [nu:3 er2] /daughter/\n"
            "玩兒 玩儿 [wan2 r5] /play/\n"
            "綠茶 绿茶 [lü4 cha2] /tea/\n"
            "玩兒 玩儿 [wan2 r5] /duplicate/\n"
            "壞例 坏例 [huai4] /bad/\n")
        self.assertEqual(normal["女儿"], {"nv'er": 1})
        self.assertEqual(normal["绿茶"], {"lv'cha": 3})
        self.assertEqual(erhua["玩儿"], {"wan'r": 2})
        self.assertNotIn("wan'er", erhua["玩儿"])
        self.assertNotIn("玩儿", normal)
        self.assertEqual(stats["unsupported_reading"], 1)
        with self.assertRaises(ValueError):
            build.parse_cedict("bad source line")

    def test_weights_and_max_dedup(self):
        self.assertEqual([build.df_weight(n) for n in (0, 1, 2, 3, 7)],
                         [1, 129, 129, 257, 385])
        self.assertEqual(build.df_weight(2 ** 100), 4096)
        self.assertEqual(build.read_tsv(b"wei'xin\tword\t5\nwei'xin\tword\t2\n"),
                         {("wei'xin", "word"): 5})


class PipelineTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.outputs = build.generate()

    def test_outputs_and_net_increment(self):
        manifest = json.loads(self.outputs[build.BASE / "manifest.json"])
        self.assertGreater(manifest["additional_to_default"], 20000)
        self.assertEqual(len([p for p in self.outputs
                              if p.suffix == ".tsv" and p.stem not in ("provenance", "rejected")]), 13)
        for name, entry in manifest["files"].items():
            data = self.outputs[build.BASE / name]
            self.assertEqual(entry["sha256"], build.lex.digest(data))
            self.assertEqual(entry["bytes"], len(data))
            if "pairs" in entry:
                self.assertEqual(entry["pairs"], len(build.read_tsv(data)))
        for name, data in self.outputs.items():
            self.assertEqual((build.ROOT / name).read_bytes(), data, str(name))

    def test_brand_and_erhua_evidence(self):
        all_pairs = {}
        for path, data in self.outputs.items():
            if path.suffix == ".tsv" and path.stem not in ("provenance", "rejected"):
                all_pairs.update(build.read_tsv(data))
        for pair in (("wei'xin", "微信"), ("fa'la'li", "法拉利"),
                     ("mai'dang'lao", "麦当劳")):
            self.assertIn(pair, all_pairs)
        erhua = build.read_tsv(self.outputs[build.BASE / "erhua.tsv"])
        self.assertIn(("wan'r", "玩儿"), erhua)
        self.assertTrue(all(w.endswith("儿") and k.split("'")[-1] == "r" for k, w in erhua))

    def test_check_is_read_only(self):
        before = {p: (build.ROOT / p).stat().st_mtime_ns for p in self.outputs}
        with patch.object(build, "generate", return_value=self.outputs):
            with patch("sys.argv", ["build_categories.py", "--check"]):
                build.main()
        self.assertEqual(before, {p: (build.ROOT / p).stat().st_mtime_ns for p in self.outputs})
        path = build.BASE / "IT.tsv"
        with patch.object(build, "generate", return_value={path: b"mismatch\n"}):
            with patch("sys.argv", ["build_categories.py", "--check"]):
                with self.assertRaises(SystemExit):
                    build.main()
        self.assertEqual(before, {p: (build.ROOT / p).stat().st_mtime_ns for p in self.outputs})

    def test_source_tamper_and_missing_license_fail_closed(self):
        scratch = os.environ.get("FLOWER_TEST_SCRATCH")
        if not scratch:
            self.fail("FLOWER_TEST_SCRATCH must point to this session's scratch")
        Path(scratch).mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            root = Path(directory)
            shutil.copytree(build.ROOT / build.SOURCE, root / build.SOURCE)
            build.load_thu(root)
            path = root / build.SOURCE / "THUOCL_IT.txt"
            path.write_bytes(path.read_bytes() + b"\n")
            with self.assertRaisesRegex(ValueError, "hash mismatch"):
                build.load_thu(root)
            shutil.copyfile(build.ROOT / build.SOURCE / "THUOCL_IT.txt", path)
            (root / build.SOURCE / "LICENSE").unlink()
            with self.assertRaises(OSError):
                build.load_thu(root)


if __name__ == "__main__":
    unittest.main()
