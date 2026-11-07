# Flower category lexicon attribution

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
