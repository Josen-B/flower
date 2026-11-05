# Flower 0.4.0 验证记录

## 用户反馈与本轮范围

用户已在另一台 Windows 机器确认 0.3.0 能输入大部分常用字。本轮针对：
自定义安装目录误拒绝、仓库 README 过度展开实现、常用字排序、整词首字母输入。
功能契约见 `design/input-and-installation.md`，使用与管理入口见 `docs/distribution.md`。
基线为 `2b9355632e4040261112a827156c7f3d23ba38e8` 上的工作区差异，未提交 Git。

## 实现与自动验证

- 新建自定义目录在创建时直接使用受保护 DACL，已有目录不修改权限。
  缺失目标的现有组件只按祖先检查；卷根 DELETE 不等于删除子项，仅已验证本地固定
  卷根祖先忽略该位，普通祖先仍拒绝 DELETE，所有路径继续拒绝子项删除及改 ACL/owner。
  4 项目录策略/原生内存 SDDL 检查通过，不改文件系统 ACL；注册解析与边界 24 项通过。
- 单字频率复用固定 jieba MIT 快照，只按精确文字匹配；11,579 个字命中频率。
  保留优先读音、其他基线读音、扩展读音的严格三档和所有 52,841 条映射。
  离线转换 12 项通过，生成及 `--check` 通过。真实 lookup 验证
  `de/yi/shi/wo/ni` 首项为“的、一、是、我、你”，不将 `yi→也` 次读音推到首位。
- 整词简拼预建共享索引，overlay/学习快照同步；comment 和学习使用完整规范读音。
  质量 4 项、句子/简拼状态 15 项、额外真实常用字与简拼 2 项通过。
  `nh` 含“你好”，按真实词频排序，可翻页；不声称必定第一候选。
- 库原 48 项、候选 18 项、router 5 项、CLI 3 项通过。
  拼音会话原 17 项中 16 项通过，1 项因 `xyz` 新增简拼可匹配“信用证”而不再是无解输入；
  改用明确的小词典验证原样提交后该单例通过，未取消无解原文契约。
- 在原生 STA/TSF 隔离 Context 回归增加 `NH` 预编辑、空格提交“你好”，真实编辑读回通过。
  该测试不全局注入，不写用户文档，不证明真实宿主激活或候选绘制。
- Python package 24 项中 23 通过、1 项符号链接权限跳过；installer 20 项通过。
  严格全目标 Clippy、格式检查通过；格式首次失败只涉及新代码换行，机械修正后通过。
  上述为分批证据复用，不是一次全套命令通过；旧 LNK4104 PRIVATE 导出警告仍存在。
- 目录/安装器独立审查及卷根例外补审通过；简拼、数据排序及消费者独立静态审查通过。
  审查不代替测试，不外推管理员目录创建或安装卸载结果。

主要命令：

```powershell
cargo test --locked --lib --bin flower-register --bin flower-settings --test candidate_popup --test common_ranking --test engine_router --test lexicon_quality --test pinyin_sentence --test pinyin_session --test registration
cargo test --locked --test pinyin_session space_commits_raw_pinyin_when_no_candidate_exists -- --exact
cargo test --locked --test registration
cargo test --locked --lib native_context_accepts_pinyin_in_write_edit_session
cargo test --locked --lib platform::install_path::tests
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

## 性能

固定 release 开发负载（包含 126 字节密集输入）：
冷加载 98.897 ms；精确 lookup P95 0.4108 ms；引擎逐键 P95 8.3129 ms，
最大 10.8951 ms；空 router clone 均值 53 ns。
这不是 TSF/绘制端到端时延，也不是多宿主内存或通用硬件保证。

## 制品

真实 MSVC、Inno Setup 6.7.3 编译成功，仍为未签名测试包。

| 文件 | 字节 | SHA-256 |
| --- | ---: | --- |
| `flower-0.4.0-windows-x64.zip` | 3,166,241 | `ca2c72b38fdca4767ddf7c5a2ac44ef384c56350273ee92f790330e28f7b15b7` |
| `flower-0.4.0-windows-x64-setup.exe` | 3,348,086 | `8f0fa805b3da09c03ba9e9fb8302b5a975cf8f8a9d8f51f0e3b055d16d6664a0` |

85 个 payload（含 SHA256SUMS）、全部包内外哈希、源 ZIP 关联、词组分区与完整许可、
源 README 与打包快照、DLL 内嵌本次单字/core 数据且不嵌 optional 数据均核验。
两个 EXE 的 `--help`、显式 Unicode 输出及非法标量拒绝通过；三个 PE 共 27 张图标
匹配源 ICO。0.3.0 ZIP/EXE 重新核对哈希保持不变。

使用新包中的真实 `check-install-path`，本机
`D:\Flower-0.4.0-readonly-check` 和 Program Files 下的缺失目标均通过；
检查后目标仍不存在。卷根、UNC、Windows 系统树目标正确拒绝。
未运行 `prepare-install-path`、register、任何真实用户词典命令或安装器。

## 未覆盖与另一台机器复测

- 管理员新建受保护目录及实际安装/卸载（特别是 D 盘、空格/中文路径）；
- 不同宿主的简拼候选、全拼/简拼切换、连续选择、取消及 GUI/DPI；
- 真实宿主的 input scope 与可选本地学习；
- 远端 CI、签名发行、真实磁盘满、写入中途崩溃/系统掉电耐久性。

本轮未关闭用户应用、注销、修改已有目录 ACL、机器注册、默认输入法或真实用户配置。
下一次实机反馈须记录版本、目录和宿主，不能把此处自动检查当成桌面已验收。
