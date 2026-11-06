# 固定离线 AI 资产来源与限制

本目录只保存清单、许可和来源模型卡，不保存 ONNX 权重或运行库二进制。
`manifest.json` 是构建时可信源；分发包内 `ai/manifest.json` 必须逐字一致。
其 `files` 的八条记录均来自实际下载文件的大小与 SHA256，清单不自包含。

## 来源

- 权重、词表和转换模型卡：`Xenova/macbert4csc-base-chinese`，revision
  `7ebfe81cf502576e93c844b95354840b2ecb5c28`。
  权重路径为 `onnx/model_int8.onnx`；词表路径为 `vocab.txt`。
  原始下载方式为 `https://huggingface.co/<repository>/resolve/<revision>/<path>`。
  转换模型卡原文保存在 `README-conversion.md`。
- 原权重模型卡：`shibing624/macbert4csc-base-chinese`，revision
  `615e6e09ef9a69ec487bc7c641ec3a311e2c11b9`，`README.md`。
  原文保存在 `README-model.md`，其中声明 Apache-2.0。
- `LICENSE-model.txt` 为 https://www.apache.org/licenses/LICENSE-2.0.txt 的全文。
  转换仓库没有独立 LICENSE；保留原模型卡、转换说明和 Apache 全文不等同于
  独立确认转换仓库所有资产或训练数据的权利，未声称逐一审核训练数据许可。
- ONNX Runtime 1.20.0 CPU Windows x64 来自清单所列 GitHub release ZIP。
  ZIP 为 65,562,355 字节，SHA256 为清单中实际下载后计算的 `archive_sha256`，
  上游未提供独立 digest，因此这不是独立来源的签名/摘要认证。
  `lib/onnxruntime.dll`、根 `LICENSE` 和 `ThirdPartyNotices.txt` 原样保留，
  后两者在 bundle 内分别命名 `LICENSE-onnxruntime.txt` 和
  `ThirdPartyNotices-onnxruntime.txt`。

## DLL 前置依赖

PE 静态导入表包括 KERNEL32、ADVAPI32、Windows API 集，以及
`MSVCP140.dll`、`MSVCP140_1.dll`、`VCRUNTIME140.dll`、`VCRUNTIME140_1.dll`。
没有 `onnxruntime_providers_shared.dll` 静态依赖；CPU 路径不随包加入该文件。
推理 helper 限定依赖搜索为 System32，不从 PATH/cwd 回退。
目标系统须已有适用的 x64 VC Runtime；本功能不安装或私自打包它。
未签名测试包与本地 SHA 校验不能证明发布者身份。
