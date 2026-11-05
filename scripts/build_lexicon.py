"""从固定的本地来源生成单字底库；无网络、无第三方 Python 依赖。"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE_NAMES = frozenset({
    "Unihan_Readings.txt", "kMandarin_8105.txt",
    "UNICODE-LICENSE.txt", "LICENSE-pinyin-data.txt",
})
SELECTED_FIELDS = frozenset({"kMandarin", "kHanyuPinyin", "kTGHZ2013"})
BASELINE_COUNT = 8105
JIEBA_REVISION = "67fa2e36e72f69d9134b8a1037b83fbb070b9775"
JIEBA_SHA256 = "7197c3211ddd98962b036cdf40324d1ea2bfaa12bd028e68faa70111a88e12a8"
JIEBA_LICENSE_SHA256 = "18ba0984839f85853b29fadaf992f7dba8fd0ca0fbeae34de2b8735222dc7a37"
RANKING = {
    "source": "jieba/dict.txt",
    "revision": JIEBA_REVISION,
    "sha256": JIEBA_SHA256,
    "license": "MIT",
    "license_sha256": JIEBA_LICENSE_SHA256,
    "license_path": "licenses/phrases/LICENSE-jieba.txt",
    "frequency": "exact single-character text frequency, not reading frequency; unknown=0",
    "rank": "f=0:0; f>0:min(179,8*floor(log2(f))+floor(8*(f-2^floor(log2(f)))/2^floor(log2(f))))",
    "tiers": {"primary": "220+min(rank,179)", "secondary": "100+min(rank,99)",
              "extra": "1+min(rank,98)"},
}


class LexiconBuildError(ValueError):
    pass


def normalize_pinyin(value: str) -> str:
    """Normalize annotated source readings, not arbitrary input sentences."""
    value = value.lower().replace("u:", "v")
    result: list[str] = []
    for char in unicodedata.normalize("NFD", value):
        if "a" <= char <= "z":
            result.append(char)
        elif char == "\u0308" and result and result[-1] == "u":
            result[-1] = "v"
        elif char in "\u0300\u0301\u0304\u030c\u0302":
            if not result:
                raise LexiconBuildError("读音以组合声调开头")
        else:
            raise LexiconBuildError(f"不支持的读音字符：{value!r}")
    key = "".join(result)
    if not re.fullmatch("[a-z]{1,8}", key):
        raise LexiconBuildError(f"无效读音：{value!r}")
    return key


def codepoint(token: str) -> str:
    if not re.fullmatch(r"U\+[0-9A-F]{4,6}", token):
        raise LexiconBuildError(f"无效码点：{token}")
    value = int(token[2:], 16)
    if not (0x3400 <= value <= 0x4DBF or 0x4E00 <= value <= 0x9FFF or 0xF900 <= value <= 0xFAFF
            or 0x20000 <= value <= 0x3FFFF or value == 0x3007):
        raise LexiconBuildError(f"不是允许的汉字码点：{token}")
    return chr(value)


def parse_baseline(text: str, expected: int = BASELINE_COUNT) -> dict[str, str]:
    result: dict[str, str] = {}
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        match = re.fullmatch(r"(U\+[0-9A-F]{4,6}):\s*(\S+)\s+#\s*(\S).*", line)
        if not match:
            raise LexiconBuildError(f"基线第 {number} 行格式错误")
        char = codepoint(match[1])
        if char != match[3] or char in result:
            raise LexiconBuildError(f"基线第 {number} 行字形不一致或重复")
        result[char] = normalize_pinyin(match[2])
    if len(result) != expected:
        raise LexiconBuildError(f"基线应有 {expected} 个唯一字符，实际 {len(result)}")
    return result


def parse_unihan(text: str) -> dict[str, set[str]]:
    result: dict[str, set[str]] = {}
    seen: set[tuple[str, str]] = set()
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 3:
            raise LexiconBuildError(f"Unihan 第 {number} 行字段错误")
        token, field, readings = fields
        if field not in SELECTED_FIELDS:
            continue
        char = codepoint(token)
        if (char, field) in seen:
            raise LexiconBuildError(f"Unihan 第 {number} 行字段重复")
        seen.add((char, field))
        words: list[str] = []
        for group in readings.split():
            if field == "kMandarin":
                words.extend(group.split(","))
            else:
                location, separator, values = group.partition(":")
                if not separator or not re.fullmatch(r"[0-9.,]+", location):
                    raise LexiconBuildError(f"Unihan 第 {number} 行定位字段错误")
                words.extend(values.split(","))
        if not words:
            raise LexiconBuildError(f"Unihan 第 {number} 行缺少读音")
        result.setdefault(char, set()).update(normalize_pinyin(word) for word in words)
    if not result:
        raise LexiconBuildError("Unihan 没有可用读音")
    return result


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def frequency_rank(frequency: int) -> int:
    if frequency < 0:
        raise LexiconBuildError("负词频")
    if frequency == 0:
        return 0
    exponent = frequency.bit_length() - 1
    bucket = 1 << exponent
    return min(179, 8 * exponent + 8 * (frequency - bucket) // bucket)


def load_frequencies(root: Path) -> dict[str, int]:
    prefix = root / "data/pinyin/phrases"
    manifest = json.loads((prefix / "manifest.json").read_text(encoding="utf-8"))
    if manifest["schema_version"] != 1 or manifest["jieba_revision"] != JIEBA_REVISION:
        raise LexiconBuildError("jieba 来源版本不一致")
    contents = {}
    for name, expected in (("dict.txt", JIEBA_SHA256),
                           ("LICENSE-jieba.txt", JIEBA_LICENSE_SHA256)):
        entry = manifest["files"][name]
        path = prefix / "sources" / name
        if path.is_symlink() or path.stat().st_size > 32 * 1024 * 1024:
            raise LexiconBuildError("不安全的 jieba 来源")
        contents[name] = path.read_bytes()
        if (entry["license"] != "MIT" or entry["sha256"] != expected
                or digest(contents[name]) != expected):
            raise LexiconBuildError(f"jieba 来源校验失败：{name}")
    result = {}
    for number, line in enumerate(contents["dict.txt"].decode("utf-8").splitlines(), 1):
        fields = line.split()
        if len(fields) != 3 or not re.fullmatch("[0-9]+", fields[1]):
            raise LexiconBuildError(f"jieba 第 {number} 行格式错误")
        word, frequency, _ = fields
        if len(word) == 1:
            if word in result:
                raise LexiconBuildError(f"jieba 单字重复：{word}")
            result[word] = int(frequency)
    return result


def load_sources(root: Path) -> tuple[dict, dict[str, bytes]]:
    sources = root / "data/pinyin/sources"
    manifest = json.loads((sources / "manifest.json").read_text(encoding="utf-8"))
    if (not isinstance(manifest, dict) or manifest.get("schema_version") != 1
            or manifest.get("baseline_count") != BASELINE_COUNT
            or not re.fullmatch("[0-9a-f]{40}", manifest.get("pinyin_data_revision", ""))
            or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", manifest.get("unicode_version", ""))):
        raise LexiconBuildError("来源清单版本或覆盖基线无效")
    files = manifest.get("files")
    if not isinstance(files, dict) or set(files) != SOURCE_NAMES:
        raise LexiconBuildError("来源文件白名单不一致")
    contents = {}
    for name in sorted(SOURCE_NAMES):
        path = sources / name
        if path.is_symlink() or path.stat().st_size > 32 * 1024 * 1024:
            raise LexiconBuildError(f"不安全或过大的来源文件：{name}")
        contents[name] = path.read_bytes()
        if digest(contents[name]) != files[name]["sha256"]:
            raise LexiconBuildError(f"来源文件校验失败：{name}")
    if b"UNICODE LICENSE V3" not in contents["UNICODE-LICENSE.txt"]:
        raise LexiconBuildError("Unicode 许可正文缺失")
    if b"The MIT License" not in contents["LICENSE-pinyin-data.txt"]:
        raise LexiconBuildError("pinyin-data 许可正文缺失")
    return manifest, contents


def generate(root: Path = ROOT) -> dict[str, bytes]:
    manifest, files = load_sources(root)
    raw = files["Unihan_Readings.txt"].decode("utf-8")
    if f"# Unicode Version {manifest['unicode_version']}" not in raw:
        raise LexiconBuildError("Unihan 内容与版本不一致")
    baseline = parse_baseline(files["kMandarin_8105.txt"].decode("utf-8"))
    readings = parse_unihan(raw)
    frequencies = load_frequencies(root)
    # Baseline readings are separately attributed data, not invented fallbacks.
    for char, preferred in baseline.items():
        readings.setdefault(char, set()).add(preferred)
    rows = []
    for char, keys in readings.items():
        for key in keys:
            rank = frequency_rank(frequencies.get(char, 0))
            weight = (220 + rank if key == baseline.get(char) else 100 + min(rank, 99)) \
                if char in baseline else 1 + min(rank, 98)
            rows.append((key, char, weight))
    rows.sort(key=lambda row: (row[0], -row[2], row[1]))
    output = (
        "# Generated by scripts/build_lexicon.py; do not edit.\n"
        "# Unicode-3.0 and MIT: see sources/manifest.json and accompanying licenses.\n"
        "# Reading-priority tiers with compressed jieba text frequency; see coverage.json.\n"
        + "".join(f"{key}\t{char}\t{weight}\n" for key, char, weight in rows)
    ).encode("utf-8")
    coverage = {
        "schema_version": 1,
        "baseline_source": "pinyin-data/kMandarin_8105.txt (upstream transcription)",
        "baseline_expected": BASELINE_COUNT,
        "baseline_covered": len(set(baseline) & set(readings)),
        "missing_characters": sorted(set(baseline) - set(readings)),
        "unique_characters": len(readings),
        "pinyin_character_pairs": len(rows),
        "pinyin_keys": len({row[0] for row in rows}),
        "supplementary_characters": sum(ord(char) > 0xFFFF for char in readings),
        "unicode_version": manifest["unicode_version"],
        "pinyin_data_revision": manifest["pinyin_data_revision"],
        "source_sha256": {name: digest(data) for name, data in sorted(files.items())},
        "base_lexicon_sha256": digest(output),
        "ranking": RANKING,
        "frequency_characters_matched": len(set(readings) & set(frequencies)),
    }
    if coverage["missing_characters"]:
        raise LexiconBuildError("基线存在缺字，拒绝生成")
    return {
        "data/pinyin/base_lexicon.tsv": output,
        "data/pinyin/coverage.json": (json.dumps(coverage, ensure_ascii=False, indent=2,
                                                sort_keys=True) + "\n").encode("utf-8"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="只校验来源和生成文件，不写文件")
    args = parser.parse_args()
    try:
        outputs = generate(ROOT)
        for name, data in outputs.items():
            target = ROOT / name
            if args.check:
                if not target.is_file() or target.read_bytes() != data:
                    raise LexiconBuildError(f"生成文件不一致：{name}")
            else:
                target.write_bytes(data)
        print("单字库与覆盖报告校验通过。" if args.check else "已生成单字库与覆盖报告。")
        return 0
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"词库构建失败：{error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
