# Third-party notices

## librime

- Upstream: https://github.com/rime/librime
- Version: 1.17.0
- Pinned release commit: `33e78140250125871856cdc5b42ddc6a5fcd3cd4`
- License: BSD
- License text: `third_party/librime/LICENSE`

The librime submodule pins its build dependencies recursively. Their individual
license files remain in their respective submodules and must be included in
any redistributed package.

## rime-pinyin-simp

- Upstream: https://github.com/rime/rime-pinyin-simp
- Pinned commit: `0c6861ef7420ee780270ca6d993d18d4101049d0`
- License: Apache-2.0
- License text: `third_party/rime-pinyin-simp/LICENSE`

Flower uses the upstream `pinyin_simp.dict.yaml` data file and supplies its own
minimal schema. No Chatterfly or Sogou lexicon/model data is included.
