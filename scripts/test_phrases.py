"""Targeted phrase source, normalization and quality checks."""
import json
import unittest

from scripts import build_phrases as build


class PhraseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.outputs = build.generate()

    def test_offline_reproducibility(self):
        for name, data in self.outputs.items():
            self.assertEqual((build.ROOT / name).read_bytes(), data)

    def test_format_bounds_sort(self):
        for name in ("phrase_lexicon.tsv", "proper_lexicon.tsv"):
            self.check_format(self.outputs[build.BASE / name])

    def check_format(self, data):
        self.assertNotIn(b"\r", data)
        rows = []
        for line in data.decode().splitlines():
            key, text, weight = line.split("\t")
            self.assertRegex(key, r"^[a-z]{1,8}('[a-z]{1,8}){1,7}$")
            self.assertEqual(len(key.split("'")), len(text))
            self.assertTrue(build.han(text))
            self.assertTrue(0 < int(weight) <= 0xffffffff)
            rows.append((key, text, int(weight)))
        self.assertEqual(rows, sorted(rows, key=lambda r: (r[0], -r[2], r[1])))
        self.assertEqual(len(rows), len({(k, t) for k, t, _ in rows}))

    def test_quality(self):
        report = json.loads(self.outputs[build.BASE / "coverage.json"])
        self.assertGreater(report["phrase_pairs"], 50000)
        self.assertEqual(report["quality"]["top1"], 12)
        self.assertEqual(report["quality"]["top5"], 12)
        self.assertEqual(len(report["quality"]["cases"]), 12)
        self.assertTrue(all(c["partition"] == "core" for c in report["quality"]["cases"]))

    def test_partition_threshold_and_general_precedence(self):
        pairs = [("a'a", "甲甲"), ("b'b", "乙乙"), ("c'c", "丙丙"),
                 ("d'd", "丁丁")]
        rows = dict(zip(pairs, (1, 999, 1000, 1)))
        core, optional = build.partition(rows, {
            "general": {pairs[0], pairs[3]}, "proper": set(pairs[1:])})
        self.assertEqual(set(core), {pairs[0], pairs[2], pairs[3]})
        self.assertEqual(optional, {pairs[1]: 999})
        self.assertEqual(core | optional, rows)
        self.assertFalse(core.keys() & optional.keys())

    def test_partition_production(self):
        def read(name):
            return {(key, text): int(weight) for key, text, weight in
                    (line.split("\t") for line in
                     self.outputs[build.BASE / name].decode().splitlines())}
        core, optional = read("phrase_lexicon.tsv"), read("proper_lexicon.tsv")
        groups = json.loads(self.outputs[build.BASE / "source-groups.json"])
        groups = {g: {tuple(pair) for pair in pairs} for g, pairs in groups.items()}
        self.assertEqual((core, optional), build.partition(core | optional, groups))
        self.assertFalse(core.keys() & optional.keys())
        report = json.loads(self.outputs[build.BASE / "coverage.json"])
        self.assertEqual(report["core_pairs"], len(core))
        self.assertEqual(report["optional_pairs"], len(optional))
        self.assertEqual(report["phrase_pairs"], len(core) + len(optional))
        self.assertEqual(report["phrase_lexicon_sha256"],
                         build.digest(self.outputs[build.BASE / "phrase_lexicon.tsv"]))
        self.assertEqual(report["proper_lexicon_sha256"],
                         build.digest(self.outputs[build.BASE / "proper_lexicon.tsv"]))

    def test_real_readings_no_cartesian_product(self):
        rows, _, stats = build.convert(
            "重慶 重庆 [Chong2 qing4] /Chongqing/\n"
            "重慶 重庆 [Chong2 qing4] /duplicate/\n", {"重庆": 100})
        self.assertEqual(rows, {("chong'qing", "重庆"): 100})
        self.assertEqual(stats["normalized_duplicates"], 1)
        self.assertNotIn(("zhong'qing", "重庆"), rows)

    def test_umlaut(self):
        rows, _, _ = build.convert("女兒 女儿 [nu:3 er2] /daughter/\n", {"女儿": 4})
        self.assertIn(("nv'er", "女儿"), rows)
        self.assertEqual(build.normalize_pinyin("nǚ"), "nv")

    def test_bad_source_line(self):
        with self.assertRaises(ValueError):
            build.convert("broken", {})

    def test_bad_jieba(self):
        for line in ("词 -1 n", "词 4294967296 n", "bad"):
            with self.assertRaises(ValueError):
                build.parse_jieba(line)

    def test_unsupported_and_length(self):
        rows, _, stats = build.convert(
            "學習 学习 [xue2 xi2] /study/\n"
            "學習 学习 [xue2] /bad alignment/\n", {"学习": 1})
        self.assertEqual(len(rows), 1)
        self.assertEqual(stats["unsupported_readings"], 1)


if __name__ == "__main__":
    unittest.main()
