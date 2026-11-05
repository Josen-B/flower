# Flower 单字底库来源

此处为开发侧离线转换快照，不在服务激活或输入时联网。

- `Unihan_Readings.txt` 来自 Unicode 17.0.0 的 `Unihan.zip`，只使用
  `kMandarin`、`kHanyuPinyin`、`kTGHZ2013` 字段。原数据版权：
  © 2025 Unicode®, Inc.；附带 `UNICODE-LICENSE.txt`（Unicode License V3，
  Copyright © 1991-2026 Unicode, Inc.）。
- `kMandarin_8105.txt` 来自 `mozillazg/pinyin-data` 固定提交
  `fe29ad32bdf352b31a02b9936e537f575432e9a6`，作为 8105 字转录检查集和优先读音，
  不是本项目声称的官方认证字表。Copyright (c) 2016 mozillazg；
  完整 MIT 正文见 `LICENSE-pinyin-data.txt`。
- 原始下载地址、各文件哈希与上游 ZIP 哈希见 `manifest.json`。
  不使用该仓库混合其他来源的 `pinyin.txt`，也不引入 CC-CEDICT 或网站字典数据。
- `scripts/build_lexicon.py` 将有实际读音的单字转换为无声调拼音，
  `ü/u:` 记作 `v`。保留优先读音、其他基线读音、扩展读音三档，
  档内使用固定 jieba 快照的精确单字文字频率（不是读音频率）。
  以纯整数 log2 分桶压缩；确切公式、提交与 SHA-256 见 `../coverage.json`。
- 词频复用 `../phrases/sources/dict.txt`，来自 fxsjy/jieba 提交
  `67fa2e36e72f69d9134b8a1037b83fbb070b9775`，Copyright (c) 2013 Sun Junyi。
  字节哈希及完整 MIT 许可由已有 `../phrases/manifest.json` 核验；
  分发包复用 `licenses/phrases/LICENSE-jieba.txt`。不复制原始词典到运行包，
  不使用 CC-CEDICT 数据生成单字库。
- 分发包附带生成词库、8105 检查集、来源清单和覆盖报告。8 MB 原始 Unicode
  快照保留在源码仓库供重建，不是已安装程序的运行依赖；两份完整许可及本说明
  放入分发包 `licenses/dictionaries/`。

多音字与扩展字受所选字段覆盖限制；没有确定读音的字不编造拼音。
收录不代表所有 Windows 字体都能显示相应字形。
