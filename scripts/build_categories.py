"""Rebuild default category lexicons from byte-locked local snapshots, offline."""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import gzip
import io
import json
from pathlib import Path
import re

try:
    from . import build_lexicon as lex
    from . import build_phrases as phrases
except ImportError:
    import build_lexicon as lex
    import build_phrases as phrases

ROOT = Path(__file__).resolve().parent.parent
BASE = Path("data/pinyin/categories")
SOURCE = Path("data/pinyin/sources/thuocl")
REVISION = "a30ce79d895d01ab5132a5c74c29703ff7efb4cc"
CATEGORIES = ("IT", "animal", "caijing", "car", "chengyu", "diming",
              "food", "law", "lishimingren", "medical", "poem")
LICENSE_SHA = "db4b8cca414db4cf487b933d07a0514f77caf44f9f3d392f2bc9d7ba0d511c58"
README_SHA = "388f9d0351d511bcd2a5ecf714b762d822aabc375ece710aae98687eb0d5bcc3"
DEFAULTS = ("data/pinyin/base_lexicon.tsv",
            "data/pinyin/phrases/phrase_lexicon.tsv",
            "data/pinyin/everyday_phrases.tsv")


def basic_word(word):
    return 2 <= len(word) <= 16 and all("\u4e00" <= c <= "\u9fff" for c in word)


def read_checked(path, expected):
    if path.is_symlink() or path.stat().st_size > phrases.MAX_BYTES:
        raise ValueError(f"unsafe source: {path}")
    data = path.read_bytes()
    if lex.digest(data) != expected:
        raise ValueError(f"hash mismatch: {path}")
    return data


def load_thu(root):
    manifest = json.loads((root / SOURCE / "manifest.json").read_text("utf-8"))
    names = {"LICENSE", "README.md"} | {f"THUOCL_{c}.txt" for c in CATEGORIES}
    if (manifest["schema_version"] != 1 or manifest["revision"] != REVISION
            or set(manifest["files"]) != names
            or manifest["files"]["LICENSE"]["sha256"] != LICENSE_SHA
            or manifest["files"]["README.md"]["sha256"] != README_SHA):
        raise ValueError("invalid THUOCL source manifest")
    files = {}
    for name in sorted(names):
        entry = manifest["files"][name]
        if entry["license"] != "MIT":
            raise ValueError("invalid THUOCL license")
        files[name] = read_checked(root / SOURCE / name, entry["sha256"])
    if b"Copyright (c) 2018 THUNLP" not in files["LICENSE"]:
        raise ValueError("missing THUOCL attribution")
    return manifest, files


def parse_cedict(text):
    """Return normal and explicitly attested r-suffix readings, with line evidence."""
    normal, erhua = defaultdict(dict), defaultdict(dict)
    stats = Counter()
    for number, line in enumerate(text.splitlines(), 1):
        if not line or line.startswith("#"):
            continue
        stats["source_entries"] += 1
        match = re.fullmatch(r"(\S+) (\S+) \[([^\]]+)\] /(.+)/", line)
        if not match:
            raise ValueError(f"invalid CC-CEDICT line {number}")
        _, word, reading, _ = match.groups()
        if not basic_word(word):
            stats["unsupported_text"] += 1
            continue
        tokens = reading.split()
        if len(tokens) != len(word) or any(
                not re.fullmatch(r"[A-Za-züÜ:]+[1-5]", t) for t in tokens):
            stats["unsupported_reading"] += 1
            continue
        key = "'".join(lex.normalize_pinyin(t[:-1]) for t in tokens)
        if "r" in key.split("'"):
            if word.endswith("儿") and key.split("'")[-1] == "r" and "r" not in key.split("'")[:-1]:
                erhua[word].setdefault(key, number)
                stats["accepted_erhua_entries"] += 1
            else:
                stats["unsupported_reading"] += 1
        else:
            normal[word].setdefault(key, number)
            stats["accepted_normal_entries"] += 1
    return normal, erhua, stats


def parse_thu_line(line):
    if not line:
        return None, "blank"
    fields = line.split("\t")
    if len(fields) != 2:
        return None, "invalid_fields"
    # Upstream IT/caijing/chengyu use spaces around the tab delimiter.
    # Strip field padding only, never remove or repair characters within DF.
    word, raw_df = (field.strip(" ") for field in fields)
    if not re.fullmatch(r"[0-9]+", raw_df):
        return None, "invalid_df"
    if not basic_word(word):
        return None, "unsupported_text"
    return (word, int(raw_df)), None


def reliable_readings(word, cedict, characters):
    if word in cedict:
        return cedict[word], "cedict"
    sets = [characters.get(c, set()) for c in word]
    if any(not s for s in sets):
        return {}, "missing_character_reading"
    if any(len(s) != 1 for s in sets):
        return {}, "ambiguous_character_reading"
    return {"'".join(next(iter(s)) for s in sets): None}, "base_monophonic"


def df_weight(df):
    return min(4096, 1 + 128 * ((df + 1).bit_length() - 1))


def read_tsv(data):
    result = {}
    for line in data.decode("utf-8").splitlines():
        if not line or line.startswith("#"):
            continue
        key, word, weight = line.split("\t")
        pair = key, word
        result[pair] = max(int(weight), result.get(pair, 0))
    return result


def generate(root=ROOT):
    thu_manifest, thu = load_thu(root)
    # Reuse authoritative builders to validate all original source bytes/licenses
    # and establish that the unchanged default tables still match those sources.
    base_outputs = lex.generate(root)
    phrase_outputs = phrases.generate(root)
    for name, data in {**base_outputs, **phrase_outputs}.items():
        if (root / name).read_bytes() != data:
            raise ValueError(f"original generated table differs: {name}")
    frequency_manifest = json.loads((root / phrases.BASE / "manifest.json").read_text("utf-8"))
    phrase_files = {n: read_checked(root / phrases.BASE / "sources" / n, e["sha256"])
                    for n, e in frequency_manifest["files"].items()}
    with gzip.GzipFile(fileobj=io.BytesIO(phrase_files["cedict.txt.gz"])) as stream:
        raw = stream.read(phrases.MAX_BYTES + 1)
    if len(raw) > phrases.MAX_BYTES or lex.digest(raw) != frequency_manifest["cedict_uncompressed_sha256"]:
        raise ValueError("invalid CEDICT decompressed bytes")
    normal, erhua, cedict_stats = parse_cedict(raw.decode("utf-8"))
    exact_readings = {word: dict(readings) for word, readings in normal.items()}
    for word, readings in erhua.items():
        exact_readings.setdefault(word, {}).update(readings)
    frequencies = phrases.parse_jieba(phrase_files["dict.txt"].decode("utf-8"))
    defaults, baseline = {}, {}
    for name in DEFAULTS:
        data = (root / name).read_bytes()
        defaults.update(read_tsv(data))
        baseline[name] = {"sha256": lex.digest(data), "bytes": len(data)}
    characters = defaultdict(set)
    for key, char in read_tsv(base_outputs["data/pinyin/base_lexicon.tsv"]):
        characters[char].add(key)
    outputs, category_stats = {}, {}
    provenance = ["category\tsource_line\ttext\tdf\tpinyin\treading_source\tcedict_line\n"]
    rejected = ["category\tsource_line\treason\traw_json\n"]
    all_pairs = {}
    for category in CATEGORIES:
        rows, stats = {}, Counter()
        for number, line in enumerate(thu[f"THUOCL_{category}.txt"].decode("utf-8").splitlines(), 1):
            stats["raw_lines"] += 1
            parsed, reason = parse_thu_line(line)
            if reason is None:
                word, df = parsed
                readings, origin = reliable_readings(word, exact_readings, characters)
                if not readings:
                    reason = origin
            if reason is not None:
                stats["rejected_" + reason] += 1
                rejected.append(f"{category}\t{number}\t{reason}\t{json.dumps(line, ensure_ascii=False)}\n")
                continue
            stats["accepted_source_entries"] += 1
            stats["reading_" + origin] += 1
            stats["zero_df_entries"] += df == 0
            for key, cedict_line in readings.items():
                pair = key, word
                weight = frequencies[word] if word in frequencies else df_weight(df)
                rows[pair] = max(weight, rows.get(pair, 0))
                provenance.append(f"{category}\t{number}\t{word}\t{line.split(chr(9))[1]}\t{key}\t{origin}\t{cedict_line or ''}\n")
        stats["unique_pairs"] = len(rows)
        stats["additional_to_default"] = len(rows.keys() - defaults.keys())
        category_stats[category] = dict(stats)
        outputs[BASE / f"{category}.tsv"] = phrases.tsv(rows)
        for pair, weight in rows.items():
            all_pairs[pair] = max(weight, all_pairs.get(pair, 0))
    for name, source in (("cedict_supplement", normal), ("erhua", erhua)):
        rows = {(key, word): frequencies.get(word, 1)
                for word, readings in source.items() for key in readings
                if name == "erhua" or (key, word) not in defaults}
        outputs[BASE / f"{name}.tsv"] = phrases.tsv(rows)
        category_stats[name] = {"unique_pairs": len(rows),
                                "additional_to_default": len(rows.keys() - defaults.keys())}
        for (key, word), weight in rows.items():
            all_pairs[key, word] = max(weight, all_pairs.get((key, word), 0))
            provenance.append(f"{name}\t{source[word][key]}\t{word}\t\t{key}\tcedict\t{source[word][key]}\n")
    outputs[BASE / "provenance.tsv"] = "".join(provenance).encode("utf-8")
    outputs[BASE / "rejected.tsv"] = "".join(rejected).encode("utf-8")
    outputs[BASE / "coverage.json"] = phrases.encoded({
        "schema_version": 1, "categories": category_stats,
        "cedict_conversion": dict(cedict_stats),
        "global_unique_pairs": len(all_pairs),
        "additional_to_default": len(all_pairs.keys() - defaults.keys()),
        "reading_policy": "CEDICT exact simplified word; otherwise all distinct base readings must be singleton",
        "ranking": "jieba exact word weight; absent: THU min(4096,1+128*floor(log2(DF+1))); supplement/erhua 1; duplicate=max",
        "df_provenance": "provenance.tsv: original nonnegative DF per accepted source line; not comparable global frequency",
        "rejections": "rejected.tsv: exact source line and raw JSON string; no source repair",
        "cedict_reading_evidence": "provenance.tsv cedict_line: 1-based uncompressed CC-CEDICT line",
    })
    outputs[BASE / "NOTICE.md"] = NOTICE.encode("utf-8")
    unicode_manifest, _ = lex.load_sources(root)
    outputs[BASE / "manifest.json"] = phrases.encoded({
        "schema_version": 1, "default_loaded": True,
        "global_unique_pairs": len(all_pairs),
        "additional_to_default": len(all_pairs.keys() - defaults.keys()),
        "baseline": baseline,
        "sources": {"thuocl": thu_manifest, "phrases": frequency_manifest,
                    "characters": unicode_manifest},
        "files": {str(p.relative_to(BASE)).replace("\\", "/"): {
            "sha256": lex.digest(data), "bytes": len(data),
            "lines": len(data.splitlines()),
            **({"pairs": category_stats[p.stem]["unique_pairs"]} if p.suffix == ".tsv" and p.stem in category_stats else {})}
            for p, data in outputs.items()},
        "licenses": ["data/pinyin/sources/thuocl/LICENSE",
                     "data/pinyin/phrases/sources/CC-BY-SA-4.0.txt",
                     "data/pinyin/phrases/sources/LICENSE-jieba.txt",
                     "data/pinyin/sources/UNICODE-LICENSE.txt",
                     "data/pinyin/sources/LICENSE-pinyin-data.txt"],
    })
    return outputs


NOTICE = """# Flower category lexicon attribution

These are transformed data, not original corpus-frequency measurements. All 13
TSV tables are intended for default loading. Duplicate pairs take maximum weight.

THUOCL: Copyright (c) 2018 THUNLP, MIT.
Source: https://github.com/thunlp/THUOCL at a30ce79d895d01ab5132a5c74c29703ff7efb4cc.
Complete MIT permission, conditions and disclaimer: data/pinyin/sources/thuocl/LICENSE.
Original README, contributors, corpus descriptions and research citation:
data/pinyin/sources/thuocl/README.md.
使用了清华大学开放中文词库。
韩世依, 张钰晖, 马云山, 涂存超, 郭志芃, 刘知远, 孙茂松.
THUOCL：清华大学开放中文词库. 2016.
Shiyi Han, Yuhui Zhang, Yunshan Ma, Cunchao Tu, Zhipeng Guo, Zhiyuan Liu,
Maosong Sun. THUOCL: Tsinghua Open Chinese Lexicon. 2016.

CC-CEDICT contributors, maintained and distributed by MDBG:
https://www.mdbg.net/chinese/dictionary?page=cc-cedict
CC BY-SA 4.0; complete legal text:
data/pinyin/phrases/sources/CC-BY-SA-4.0.txt.
Changes: simplified field selected; basic Han 2–16; tones removed; v for ü;
apostrophe syllable boundaries; exact attested readings only; explicit final r
kept independently from er; old default pairs excluded from supplement.
The independent erhua table retains all attested final-r pairs, including
existing default pairs; global increments are counted after deduplication.
CEDICT-derived readings and their resulting adapted tables remain CC BY-SA 4.0.
No endorsement by the original contributors is implied.

jieba: Copyright (c) 2013 Sun Junyi, MIT; exact word frequencies reused.
Complete license: data/pinyin/phrases/sources/LICENSE-jieba.txt.
Unicode Unihan 17.0.0: Copyright © 1991–2025 Unicode, Inc., Unicode-3.0.
Complete license: data/pinyin/sources/UNICODE-LICENSE.txt.
pinyin-data baseline transcription: MIT; complete copyright and license:
data/pinyin/sources/LICENSE-pinyin-data.txt.
Single-character fallback uses every distinct reading of the unchanged base,
never merely its preferred reading. It is not manual verification of names.

manifest.json locks each input, generated file, default baseline and license;
coverage.json reports acceptance/rejection and net unique pairs;
provenance.tsv retains original DF and exact reading-source line evidence.
Source snapshots are available in the source checkout for fully offline rebuild.
"""


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        outputs = generate()
        for name, data in outputs.items():
            path = ROOT / name
            if args.check:
                if not path.is_file() or path.read_bytes() != data:
                    raise ValueError(f"generated output differs: {name}")
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
    except (ValueError, OSError, KeyError, TypeError) as error:
        parser.exit(1, f"{error}\n")
    print("category outputs verified" if args.check else "category outputs generated")


if __name__ == "__main__":
    main()
