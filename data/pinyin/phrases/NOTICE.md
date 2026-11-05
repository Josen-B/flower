# Phrase data attribution and redistribution

The phrase TSV, source-groups.json and coverage.json are adaptations of
CC-CEDICT, published by MDBG and maintained by the CC-CEDICT community.
Referenced work: CEDICT, Copyright (C) 1997, 1998 Paul Andrew Denisowski.
Source: https://www.mdbg.net/chinese/dictionary?page=cc-cedict
Snapshot: 2026-11-04T07:19:40Z. No endorsement is implied.

CC-CEDICT and these derived data are distributed under Creative Commons
Attribution-ShareAlike 4.0 International:
https://creativecommons.org/licenses/by-sa/4.0/
The complete legal text is in sources/CC-BY-SA-4.0.txt. Preserve attribution,
license links, and modification notices; distribute adaptations under the same
license and do not impose additional restrictions. This is a separate data
license, not a relicensing of Flower's program code.

Weights and vocabulary intersection use fxsjy/jieba's official jieba/dict.txt,
Copyright (c) 2013 Sun Junyi, MIT. The complete permission and disclaimer are
in sources/LICENSE-jieba.txt. Revision and exact URLs/hashes are in manifest.json.

Flower modifications: select original simplified-form fields of 2–8 Han
characters with positive jieba dictionary frequency; retain only attested
one-syllable-per-character numeric-tone readings; lowercase, strip tone and
normalize ü/u: to v; retain syllable boundaries; merge duplicate normalized
(reading, text) pairs; attach the unchanged jieba word frequency; stable sort.
No traditional-to-simplified conversion, inferred pronunciation, single-character
Cartesian product, upstream code execution or fresh corpus frequency measurement.
All readings of one word share its dictionary frequency.

source-groups.json records CC-CEDICT's capitalized proper-name annotations,
separately from uncapitalized readings. A pair may belong to both groups.
This is not exhaustive name/place/technical-domain classification.
phrase_lexicon.tsv is the default core: all general mappings, plus proper
mappings with unchanged jieba frequency at least 1000. General membership takes
precedence when a pair has both annotations. Remaining proper-only mappings
are in proper_lexicon.tsv, an optional extension for explicit user import,
not automatic loading. No pair is duplicated across the two TSV files.
Neither partition is represented as a fully manually audited vocabulary.

Source-checkout paths above refer to data/pinyin/phrases/. In a Flower
distribution, this notice and the complete CC BY-SA / jieba MIT texts are
together in licenses/phrases/; the pinned source manifest and generated TSVs
are in data/pinyin/phrases/. The raw snapshots remain available in the source
checkout for reproducible offline rebuilding.

quality-cases.json is a small reproducible developer regression set, not an
unbiased accuracy benchmark. coverage.json ranks exact joined-pinyin matches
in the default core using dictionary weights and stable text order; each case
also records its actual partition. It does not demonstrate engine
integration, sentence decoding, TSF input or desktop behavior.

Offline reproduction: `python scripts/build_phrases.py`; read-only comparison:
`python scripts/build_phrases.py --check`. Standard library only; no downloads.
Retain source snapshots and both license texts for offline reconstruction.
