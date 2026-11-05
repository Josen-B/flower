"""Offline CC-CEDICT/jieba intersection; Python standard library only."""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import re
from collections import Counter, defaultdict
from pathlib import Path

try:
    from .build_lexicon import normalize_pinyin
except ImportError:
    from build_lexicon import normalize_pinyin

ROOT = Path(__file__).resolve().parent.parent
BASE = Path("data/pinyin/phrases")
NAMES = {"cedict.txt.gz", "dict.txt", "CC-BY-SA-4.0.txt", "LICENSE-jieba.txt"}
MAX_BYTES = 32 * 1024 * 1024


def digest(data):
    return hashlib.sha256(data).hexdigest()


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()


def han(word):
    return all(0x3400 <= ord(c) <= 0x9fff or 0x20000 <= ord(c) <= 0x323af
               or c == "〇" for c in word)


def parse_jieba(text):
    frequencies = {}
    for number, line in enumerate(text.splitlines(), 1):
        fields = line.split()
        if len(fields) != 3 or not re.fullmatch(r"[0-9]+", fields[1]):
            raise ValueError(f"invalid jieba line {number}")
        word, frequency, _ = fields
        weight = int(frequency)
        if not 0 <= weight <= 0xffffffff:
            raise ValueError("jieba frequency outside u32")
        frequencies[word] = max(weight, frequencies.get(word, 0))
    return frequencies


def convert(text, frequencies):
    rows = {}
    groups = defaultdict(set)
    stats = Counter()
    for number, line in enumerate(text.splitlines(), 1):
        if line.startswith("#") or not line:
            continue
        match = re.fullmatch(r"(\S+) (\S+) \[([^\]]+)\] /(.+)/", line)
        if not match:
            raise ValueError(f"invalid CC-CEDICT line {number}")
        _, word, reading, gloss = match.groups()
        stats["source_entries"] += 1
        if not (2 <= len(word) <= 8 and han(word) and frequencies.get(word, 0)):
            stats["outside_intersection"] += 1
            continue
        tokens = reading.split()
        if len(tokens) != len(word) or any(
                not re.fullmatch(r"[A-Za-züÜ:]+[1-5]", token) for token in tokens):
            stats["unsupported_readings"] += 1
            continue
        key = "'".join(normalize_pinyin(token[:-1]) for token in tokens)
        pair = key, word
        stats["accepted_source_entries"] += 1
        if pair in rows:
            stats["normalized_duplicates"] += 1
        rows[pair] = frequencies[word]
        # Source annotations are retained as an audit grouping, not a claim of
        # complete named-entity/domain classification.
        group = "proper" if any(t[0].isupper() for t in tokens) else "general"
        groups[group].add(pair)
    if not rows:
        raise ValueError("empty intersection")
    return rows, groups, stats


def ranked(rows, key):
    best = {}
    for (pinyin, word), weight in rows.items():
        if pinyin.replace("'", "") == key.replace("'", ""):
            best[word] = max(weight, best.get(word, 0))
    return sorted(best, key=lambda word: (-best[word], word))


def partition(rows, groups):
    core = {pair: weight for pair, weight in rows.items()
            if pair in groups["general"] or
            (pair in groups["proper"] and weight >= 1000)}
    optional = {pair: weight for pair, weight in rows.items() if pair not in core}
    return core, optional


def tsv(rows):
    order = sorted(rows, key=lambda pair: (pair[0], -rows[pair], pair[1]))
    return "".join(f"{key}\t{word}\t{rows[key, word]}\n" for key, word in order).encode()


def generate(root=ROOT):
    base = root / BASE
    manifest = json.loads((base / "manifest.json").read_text(encoding="utf-8"))
    if (manifest.get("schema_version") != 1 or set(manifest.get("files", {})) != NAMES
            or not re.fullmatch("[0-9a-f]{40}", manifest.get("jieba_revision", ""))):
        raise ValueError("invalid source manifest")
    files = {}
    for name in sorted(NAMES):
        path = base / "sources" / name
        if path.is_symlink() or path.stat().st_size > MAX_BYTES:
            raise ValueError("unsafe source")
        data = path.read_bytes()
        if digest(data) != manifest["files"][name]["sha256"]:
            raise ValueError(f"hash mismatch: {name}")
        files[name] = data
    if (b"Attribution-ShareAlike 4.0 International" not in files["CC-BY-SA-4.0.txt"]
            or b"Copyright (c) 2013 Sun Junyi" not in files["LICENSE-jieba.txt"]):
        raise ValueError("missing license")
    # Bounded decompression, including a one-byte overflow sentinel.
    import io
    with gzip.GzipFile(fileobj=io.BytesIO(files["cedict.txt.gz"])) as stream:
        raw = stream.read(MAX_BYTES + 1)
    if len(raw) > MAX_BYTES or digest(raw) != manifest["cedict_uncompressed_sha256"]:
        raise ValueError("invalid decompressed snapshot")
    text = raw.decode("utf-8")
    if ("#! date=" + manifest["cedict_snapshot"]) not in text:
        raise ValueError("snapshot date mismatch")
    rows, groups, stats = convert(text, parse_jieba(files["dict.txt"].decode("utf-8")))
    core, optional = partition(rows, groups)
    output, optional_output = tsv(core), tsv(optional)
    cases = json.loads((base / "quality-cases.json").read_text(encoding="utf-8"))
    results = []
    for case in cases:
        candidates = ranked(core, case["pinyin"])
        rank = candidates.index(case["text"]) + 1 if case["text"] in candidates else None
        if (case["pinyin"], case["text"]) not in rows:
            raise ValueError(f"quality case not in source: {case}")
        results.append({**case, "rank": rank,
                        "partition": "core" if (case["pinyin"], case["text"]) in core
                        else "optional"})
    words = Counter(word for _, word in rows)
    keys = Counter(key.replace("'", "") for key, _ in rows)
    coverage = {
        "schema_version": 1, "phrase_pairs": len(rows), "unique_words": len(words),
        "polyphonic_words": sum(n > 1 for n in words.values()),
        "ambiguous_joined_keys": sum(n > 1 for n in keys.values()),
        "conversion": dict(stats), "phrase_lexicon_sha256": digest(output),
        "proper_lexicon_sha256": digest(optional_output),
        "core_pairs": len(core), "optional_pairs": len(optional),
        "core_unique_words": len({word for _, word in core}),
        "optional_unique_words": len({word for _, word in optional}),
        "partition_policy": "core=general or proper frequency>=1000; remainder optional, not auto-loaded",
        "source_sha256": {n: digest(d) for n, d in files.items()},
        "ranking": "jieba dict.txt raw word frequencies; identical for each attested reading",
        "quality": {"cases": results, "top1": sum(c["rank"] == 1 for c in results),
                    "top5": sum(c["rank"] is not None and c["rank"] <= 5 for c in results)},
        "groups": {g: len(v) for g, v in groups.items()},
        "grouping_limit": "CC-CEDICT uppercase proper-name annotation; not exhaustive domain classification",
    }
    return {BASE / "phrase_lexicon.tsv": output, BASE / "coverage.json": encoded(coverage),
            BASE / "proper_lexicon.tsv": optional_output,
            BASE / "source-groups.json": encoded({
                g: [[k, w] for k, w in sorted(v)] for g, v in groups.items()})}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        for name, data in generate().items():
            path = ROOT / name
            if args.check:
                if not path.exists() or path.read_bytes() != data:
                    raise ValueError(f"generated output differs: {name}")
            else:
                path.write_bytes(data)
    except (ValueError, OSError, KeyError) as error:
        parser.exit(1, f"{error}\n")
    print("phrase outputs verified" if args.check else "phrase outputs generated")


if __name__ == "__main__":
    main()
