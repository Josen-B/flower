# Flower Windows x64 分发与使用

Flower 支持 Windows 10/11 x64。当前源码版本为 0.4.1，提供离线 EXE 测试安装器与
ZIP 包；测试包尚未签名，校验哈希不等于验证发布者身份。不支持 x86 或 ARM64 宿主。

## 安装、启用与升级

1. 校验下载文件旁的 `.sha256`。例如：
   ```powershell
   Get-FileHash .\flower-0.4.1-windows-x64-setup.exe -Algorithm SHA256
   ```
2. EXE 安装器请求管理员权限，仅进行机器级安装与注册，不更改默认输入法。
3. 在普通用户桌面从开始菜单运行“启用 Flower（当前用户）”，再按 `Win + Space`
   选择 Flower。提升账户不是目标普通用户，安装成功也不等于已经切换输入法。
4. 保存设置、修改词典或学习开关后，重新切换到 Flower 才会载入新快照。

安装器保守拒绝已有 Flower 注册，不支持原地自动升级。请先在使用 Flower 的用户下
停用，保存工作并正常退出加载旧 DLL 的应用，再通过旧版卸载器卸载；手工注册的版本
须从旧稳定目录用管理员终端运行 `flower-register unregister`。
不要覆盖正在使用的 DLL，不要删除未知注册表项、其他输入法或用户文件。
卸载失败或提示占用时保留现场，不强制关闭应用或删除安装树。

### 自定义目录

0.4.1 使用分组件安装。目录向导选择的是**用户文件目录**，可以是其他本地盘的深层
子路径，例如 `D:\software\Flower` 或其下更深的新目录；不再要求已有父目录为管理员所有。
此处保存设置程序、说明和程序许可证副本，由原始非提升用户创建，不覆盖已有同名文件。

实际注册的 DLL、注册工具和卸载器固定在 Program Files 下的受保护 Flower 目录，
并不跟随用户目录移动；开始菜单公共入口也指向安全系统目录。安装器明确显示两种位置，
不会为迁就任意路径而修改 D 盘根、已有 `software` 或其他父目录的权限。
系统组件仍使用严格安全检查。原始非提升用户不可用或副本部署失败时明确报错，
不退回管理员权限写入用户可替换的目录；可能保留已安装系统组件供修复或正常卸载。

用户目录仍须遵守 Windows 路径长度和访问权限限制；网络、SUBST、重解析点/联接、
系统目录和卷根不支持。建议选专属目录，不要直接选含其他软件的共享目录作为 Flower 目标。
卸载保留自选目录副本及词典，可由用户自行删除，不由提升卸载器递归清理该目录。

注册工具提供两个不同入口：

```powershell
flower-register check-install-path --path "D:\Flower"
flower-register prepare-install-path --path "D:\Flower"
```

前者只读；后者会新建缺失目录，供管理员安装流程调用，不修复已有目录。
检查失败没有复制载荷或注册的授权；准备失败可能留下本次创建的空安全目录。
这两个命令检查的是**系统组件目录**，不能用它们判断用户文件目录是否合格。

### ZIP 手工安装

ZIP 本身不提权。校验外部哈希和包内 `SHA256SUMS` 后，将文件放入受保护的稳定安装
目录，不从 Downloads、开发 `target` 或会话临时目录注册。
在管理员终端使用该稳定目录中的注册工具：

```powershell
.\flower-register.exe check-install-path --path "C:\Program Files\Flower\manual"
.\flower-register.exe register --dll "C:\Program Files\Flower\manual\flower.dll"
```

之后每个目标普通用户分别运行 `enable`。`disable` 只停用当前用户；机器级
`unregister` 不自动关闭应用，不删除 DLL、设置或其他用户配置。

## 输入和候选

- 全拼：`nihao → 你好`、`zhongguo → 中国`。`ü` 使用 `v`，如 `lv`、`nv`。
- 分音：`xi'an` 明确按 `xi + an` 查找；开头的普通引号不被当作组词吞掉。
- 简拼：整词每音节首字母，如 `nh → 你好`、`zg → 中国`、`xx → 学习`。
  `zh/ch/sh` 都取其第一个字母 `z/c/s`；不是双拼。不支持任意混合全拼简拼或
  将多个缩写自动组句。完整全拼优先，其后是整词简拼、组合和部分候选。
  日常词语使用项目维护的优先层，其内继续按原词频稳定排列；该层不是统计词频。
- `Space`、数字或鼠标选择候选。部分选词后保留同一预编辑，显示已确认中文及剩余拼音；
  全部选完才一次提交。
- `Backspace` 先删剩余拼音；剩余为空时撤回上一段选择。
- `+`（主键盘 `=` 也可）下一页，`-` 上一页；支持数字小键盘加减，
  保留 PageUp/PageDown。非组词和 Ctrl/Alt/Win 快捷键直通。
- `Escape` 取消整段未提交内容。`Enter` 确认高亮完整候选；没有完整候选时提交
  已确认中文与原始剩余拼音，不静默丢弃后缀。

输入长度上限为 128 ASCII 字节；超过上限拒绝本次修改，不截断已有输入。
离线组句使用有界静态词图，不是云端语言模型，也不保证每句话自动选对。

候选窗仅显示编号和候选文字，从左到右排列，空间不足换行，不再绘制拼音头部或
每项读音。文档内的拼音预编辑仍保留，未提交前可以继续修改。最小可读空间放不下
整页时不显示部分候选；键盘翻页和选择仍有效。

某些宿主在编辑瞬间不提供字符布局。0.4.1 会尝试关联该视图的系统光标，
再用有效视图边缘作临时位置；后者不是精确插入符位置。明确不可见/裁剪或失焦时仍隐藏。
显示使用普通窗口 Z 序，不使用永久置顶或激活窗口。Edge 和其他软件的实际效果需复测，
不能将几何和 TSF 隔离回归当作该宿主已通过。

内置 44,350 个有读音字符、52,841 条拼音—字映射，8105 字检查集逐字可达；
默认核心词组为 66,893 条词音映射，另有 12,165 条默认不加载的专名扩展。
0.4.0 使用固定 jieba 字典的文字频率改善读音档内排序，保留优先读音与全部生僻候选；
文字频率不是多音字各个读音的真实使用频率，不将次读音因文字高频提升至常用读音之前。
原 MVP 演示词条仍随源码/包供审计，但不再覆盖运行表的真实数据权重。
0.4.1 另增加人工日常词优先层，改善“你好、我们、什么、可以”等词的可达性，
不添加伪造读音，不删除其他候选，也不把人工优先级描述为测量频率。
扩展字符是否有字体可显示，与是否有读音/可查找到候选是不同问题。

## 用户词典与设置

无参数运行 `flower-settings` 打开窗口，可调整候选页大小、主题、专业词库和学习开关。
用户词典位于 `%LOCALAPPDATA%\Flower\dictionary.json`，不写入安装目录。
词条 TSV 为 UTF-8 三列：规范分音拼音、文字、正整数权重；一字一音节，例如：

```text
ni'hao	你好	1000
```

```powershell
flower-settings dictionary status
flower-settings dictionary import "$env:USERPROFILE\Documents\words.tsv"
flower-settings dictionary export "$env:USERPROFILE\Documents\words-export.tsv"
flower-settings dictionary remove "ni'hao" "你好"
flower-settings dictionary import "$env:USERPROFILE\Documents\terms.tsv" --domain "专业词"
flower-settings dictionary domain "专业词" on
flower-settings dictionary domain "专业词" off
flower-settings dictionary export "$env:USERPROFILE\Documents\terms-export.tsv" --domain "专业词"
flower-settings dictionary learning on
flower-settings dictionary learning off
flower-settings dictionary clear-learning
flower-settings unicode U+20BB7
```

导入错行整体拒绝；专业词库新导入默认停用，必须明确启用。导出使用新文件，不覆盖
已有文件。坏 JSON、未知版本或超限文件报错并保留，不静默重建。可选专名 TSV 可通过
`--domain` 导入。已启用的规范词条也参与简拼。

学习默认关闭，无遥测、无云同步。只在可靠非敏感输入范围、最终提交成功且焦点/激活
身份未过期时后台投递；密码、私密、未知范围、取消及无解原文不学习。
关闭后不再记录，下次激活也不应用旧学习权重，但保留旧记录；清空不自动关闭开关。
词典文件按跨进程锁内重读、同步临时文件、原子替换持久化；不承诺进程崩溃前尚未处理
的队列事件都已保存。外部占用可能导致明确失败，不能靠删旧文件或放宽权限绕过。
Unicode 入口仅向标准输出打印一个合法非控制字符，无换行，不向应用注入或写剪贴板。

## 构建与核验

需要 Python 3.11+、Rust stable、MSVC C++ Build Tools、Windows SDK 和
`x86_64-pc-windows-msvc`。组包需当前 Rust 工具链的 `rust-docs` 以收集标准库许可。
EXE 另需 Inno Setup 6；这些是开发依赖，目标机器不需要 Rust/Python。

```powershell
python scripts/build_lexicon.py --check
python scripts/build_phrases.py --check
python scripts/package.py
python scripts/installer.py --package dist/flower-0.4.1-windows-x64.zip
```

可以用 `--output-dir` 指定新的输出目录、`--iscc` 指定 Inno 编译器。
组包使用 `--locked`、真实 release Cargo artifact，检查 AMD64 PE32+、DLL 导出、
数据及许可，再生成单顶层 ZIP、`manifest.json`、`SHA256SUMS` 和外部 `.sha256`。
已有同名输出拒绝覆盖。安装器验证源 ZIP 后包装，另生成源 ZIP 哈希关联清单、
`.exe.sha256` 和 Inno 引擎许可；构建工具不会运行生成的安装器。

ZIP 包括 DLL、两个管理程序、本文、程序和依赖许可、单字/词组数据、来源清单及覆盖
报告。完整上游大快照保留在源码，不全部随运行包发布。包内多个 `manifest.json`
分属不同数据集；包身份应读取顶层 manifest，不能任选同名文件。

## 许可和验收边界

程序代码为 MIT OR Apache-2.0。单字读音来自固定 Unicode Unihan 与 pinyin-data，
分别保留 Unicode/MIT 许可；单字文字频率复用固定 jieba MIT 快照，许可位于
`licenses/phrases/LICENSE-jieba.txt`。CC-CEDICT 派生词组采用 CC BY-SA 4.0，词频另保留
jieba MIT 归属；完整许可和修改说明在 `licenses/phrases/`，来源清单在包内数据目录。
不能用程序根许可证替代数据许可。

用户已确认 0.3.0 能输入大部分常用字、0.4.0 可在 D 盘根下安装。0.4.1 的分组件安装、
横向候选窗、加减翻页、日常词组及缺失布局兼容定位需要另一台
Windows 机器复测；本地自动检查、PE/图标/哈希验证不能代替 GUI、真实宿主、管理员
目录创建及安装卸载验收。远端 CI 未实际运行，测试包未签名，不标作正式发行通过。
