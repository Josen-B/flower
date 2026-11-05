# 项目人工常用短语

`everyday_phrases.tsv` 由 Flower contributors 为本项目人工编写、逐条指定规范读音，
用于日常交流、问候、办事表达。未联网采集词条，不来自 CC-CEDICT 或其他上游词表，
不计入其 core/proper 覆盖数。每字一音节，含“儿”的写法保留独立 `er` 音节，
不以儿化缩写代替规范索引。

本表独立采用 MIT 许可，Copyright (c) 2026 Flower contributors。
完整许可正文随项目及分发包的 `LICENSE-MIT` 提供。
每条权重统一为 100，是项目人工权重，不是统计频率、语料计数或上游排名。
本表规范词对在 builtin 中归入项目日常层；重合的规范词对沿用去重与最高权重规则。
自定义 `from_tsv` 不启用这一分类。

固定记录数：155。
TSV SHA-256：`86aed552183db1964ca9f0c9f4a1978bd87b11b87202f60a29dea9d997312f2d`。
修改表时须同步打包器中独立的数量与哈希锁定及本说明；包内文件另受 SHA256SUMS 校验。
