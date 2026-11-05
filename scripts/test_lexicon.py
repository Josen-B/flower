"""单字库格式、锁定来源和离线重建测试；不访问网络。"""
import contextlib
import io
import json
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts import build_lexicon as lex


class LexiconSourceTests(unittest.TestCase):
    def test_readings_normalize_tones_and_umlaut(self):
        for original, expected in [
            ("nǚ", "nv"), ("lǜ", "lv"), ("lüè", "lve"), ("nu:", "nv"),
            ("zhōng", "zhong"), ("ń", "n"), ("ế", "e"), ("NǏ", "ni"),
        ]:
            self.assertEqual(lex.normalize_pinyin(original), expected)

    def test_malformed_readings_are_not_silently_fixed(self):
        for invalid in ["", "ni3", "ni hao", "xi'an", "你", "a/b", "\u0304n", "a\0b"]:
            with self.subTest(value=invalid), self.assertRaises(lex.LexiconBuildError):
                lex.normalize_pinyin(invalid)

    def test_baseline_checks_unique_count_and_codepoint_comment(self):
        row = "U+4F60: nǐ  # 你\n"
        self.assertEqual(lex.parse_baseline(row, expected=1), {"你": "ni"})
        for text, count in [(row + row, 2), (row, 2), ("U+4F60: nǐ  # 好", 1),
                            ("U+4F60 nǐ 你", 1), ("U+4DC0: nǐ  # ䷀", 1)]:
            with self.subTest(text=text), self.assertRaises(lex.LexiconBuildError):
                lex.parse_baseline(text, expected=count)

    def test_unihan_merges_actual_readings_not_cross_products(self):
        rows = (
            "U+91CD\tkMandarin\tzhòng\n"
            "U+91CD\tkHanyuPinyin\t12345.010:zhòng,chóng\n"
            "U+91CD\tkTGHZ2013\t100.010:zhòng\n"
            "U+91CD\tkDefinition\theavy\n"
        )
        self.assertEqual(lex.parse_unihan(rows), {"重": {"zhong", "chong"}})

    def test_unknown_fields_are_ignored_but_selected_bad_fields_fail(self):
        valid = "U+4F60\tkMandarin\tnǐ\n"
        for bad in [
            "U+4F60\tkMandarin\tnǐ\n",  # duplicate selected field
            "U+4F60\tkHanyuPinyin\tmissing-location\n",
            "U+4F60\tkMandarin\t\n",
            "U+110000\tkMandarin\tnǐ\n",
        ]:
            with self.subTest(bad=bad), self.assertRaises(lex.LexiconBuildError):
                lex.parse_unihan(valid + bad)
        self.assertEqual(lex.parse_unihan(valid + "U+4F60\tkDefinition\ttext\n"), {"你": {"ni"}})

    def test_real_sources_generate_deterministically_with_full_baseline(self):
        first = lex.generate()
        self.assertEqual(first, lex.generate())
        coverage = json.loads(first["data/pinyin/coverage.json"])
        self.assertEqual(coverage["baseline_covered"], 8105)
        self.assertEqual(coverage["missing_characters"], [])
        self.assertGreater(coverage["unique_characters"], 20000)
        rows = [line.split("\t") for line in first["data/pinyin/base_lexicon.tsv"].decode().splitlines()
                if not line.startswith("#")]
        self.assertEqual(len(rows), len({(key, char) for key, char, _ in rows}))
        self.assertTrue(all(len(char) == 1 for _, char, _ in rows))

    def fixture(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        shutil.copytree(lex.ROOT / "data/pinyin/sources", root / "data/pinyin/sources")
        prefix = root / "data/pinyin/phrases"
        (prefix / "sources").mkdir(parents=True)
        for name in ("manifest.json", "sources/dict.txt", "sources/LICENSE-jieba.txt"):
            shutil.copyfile(lex.ROOT / "data/pinyin/phrases" / name, prefix / name)
        return root

    def test_frequency_rank_is_integer_monotone_and_bounded(self):
        ranks = [lex.frequency_rank(value) for value in range(100000)]
        self.assertEqual(ranks, sorted(ranks))
        self.assertEqual(ranks[0], 0)
        self.assertEqual(lex.frequency_rank(3), 12)
        self.assertEqual(lex.frequency_rank(1 << 30), 179)
        self.assertLess(1 + 98, 100)
        self.assertLess(100 + 99, 220)

    def test_frequency_snapshot_and_license_corruption_fail_closed(self):
        for name in ("dict.txt", "LICENSE-jieba.txt"):
            root = self.fixture()
            (root / "data/pinyin/phrases/sources" / name).write_bytes(b"broken")
            with self.assertRaisesRegex(lex.LexiconBuildError, "校验"):
                lex.generate(root)

    def test_frequency_preserves_all_readings_and_preferred_tiers(self):
        outputs = lex.generate()
        baseline = lex.parse_baseline(
            (lex.ROOT / "data/pinyin/sources/kMandarin_8105.txt").read_text(encoding="utf-8"))
        readings = lex.parse_unihan(
            (lex.ROOT / "data/pinyin/sources/Unihan_Readings.txt").read_text(encoding="utf-8"))
        expected = {(key, char) for char, keys in readings.items() for key in keys}
        expected.update((key, char) for char, key in baseline.items())
        rows = [line.split("\t") for line in outputs["data/pinyin/base_lexicon.tsv"].decode().splitlines()
                if not line.startswith("#")]
        self.assertEqual({(key, char) for key, char, _ in rows}, expected)
        for key, char, weight in rows:
            low, high = ((220, 399) if key == baseline.get(char) else
                         (100, 199) if char in baseline else (1, 99))
            self.assertTrue(low <= int(weight) <= high)
        for key, preferred in (("yi", "一"), ("ni", "你")):
            candidates = [(char, int(weight)) for reading, char, weight in rows if reading == key]
            self.assertEqual(max(candidates, key=lambda row: row[1])[0], preferred)

    def test_source_hash_mismatch_is_rejected(self):
        root = self.fixture()
        (root / "data/pinyin/sources/kMandarin_8105.txt").write_text("broken", encoding="utf-8")
        with self.assertRaisesRegex(lex.LexiconBuildError, "校验"):
            lex.generate(root)

    def test_manifest_cannot_escape_source_whitelist(self):
        root = self.fixture()
        path = root / "data/pinyin/sources/manifest.json"
        manifest = json.loads(path.read_text())
        manifest["files"]["../../outside"] = {"sha256": "0" * 64}
        path.write_text(json.dumps(manifest), encoding="utf-8")
        with self.assertRaisesRegex(lex.LexiconBuildError, "白名单"):
            lex.generate(root)

    def test_check_mode_detects_changed_outputs_without_overwriting(self):
        root = self.fixture()
        for name, data in lex.generate(root).items():
            (root / name).write_bytes(data)
        target = root / "data/pinyin/base_lexicon.tsv"
        target.write_bytes(b"do not overwrite")
        with mock.patch.object(lex, "ROOT", root), mock.patch("sys.argv", ["build_lexicon", "--check"]), \
                contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(lex.main(), 1)
        self.assertEqual(target.read_bytes(), b"do not overwrite")


if __name__ == "__main__":
    unittest.main()
