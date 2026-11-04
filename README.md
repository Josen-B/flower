# Flower

Flower 是一款离线运行的 Windows 10/11 x64 拼音输入法。其实现是一个名为
`flower` 的单一 Rust Cargo 包。

## 当前状态

- `flower::engine` 是一个用安全 Rust 编写、与平台无关的拼音引擎。
- `flower::platform` 包含平台无关的组词/候选状态适配器以及 Windows
  TSF/COM 边界。其 `windows` 依赖仅在 Windows 目标上启用。
- 候选窗已接入 TSF 上下文：编辑会话成功后更新预编辑、编号候选、高亮和页码，
  并通过 `ITfContextView` 查询预编辑末端位置，以非激活方式显示窗口。
  提交、取消、退格至空或服务失去前台焦点时隐藏窗口；布局不可用时保持隐藏。
- 候选行和页脚共用平台无关布局。鼠标点击经窗口自定义消息延迟进入 TSF
  异步编辑会话，写入前校验激活 token、前台焦点 epoch、当前 TSF top context
  和快照 revision 后选择候选或翻页；陈旧点击不进入文档写路径。失败时只恢复
  引擎侧旧快照并隐藏窗口；TSF 的 `SetText`/`EndComposition` 不是事务，不承诺宿主文档
  自动回滚。
- 弹窗优先显示在插入符下方，放不下时移到上方并夹入目标显示器工作区；支持
  负坐标和多显示器 DPI 重算，不修改宿主进程 DPI awareness。
- `flower::settings` 提供基于 serde 的 JSON 设置模型、显式路径存储和 Windows
  当前用户存储路径。`flower-settings` 是纯 Rust/Win32 设置程序，可设置候选页
  大小及候选窗主题。
- TSF 在每次激活时为该次激活加载一次设置。新的 TSF 上下文使用配置的候选页大
  小，候选窗使用配置的主题；保存设置后重新切换到 Flower 才会生效。
- `EngineRouter` 是 TSF 层使用的平台无关边界。
- `DllGetClassObject` 公开 Flower 类工厂，并且只接受 Flower 文本服务
  CLSID。活动文本服务为每个 TSF 上下文保留一个引擎会话，并通过同步编辑
  会话处理键盘更新；鼠标选择与翻页使用异步编辑会话。
- `flower-register` 提供机器级 x64 COM/TSF 注册与当前用户启用分离流程。管理
  员使用 `register`/`unregister` 写 HKLM，普通用户使用 `enable`/`disable`
  只管理自己的 zh-CN Flower 配置。机器注册不会自动为所有用户启用输入法。
- `scripts/package.py` 提供 Windows 10/11 x64 ZIP 打包流程：严格筛选 Cargo
  release artifact，校验 AMD64 PE32+ 与 DLL 导出，收集依赖和 Rust 标准库
  许可正文，并生成确定性 ZIP、包内清单和外部 SHA-256。
- 已在 Windows x64 上生成真实未签名 ZIP，并通过包内外哈希、两个 EXE 帮助和
  COM 接口查询。桌面测试发现选择 Flower 后仍直通英文，尚未通过输入验收；
  完整上下文焦点处理和签名安装器也仍待完成。
  详见[本机验收记录](docs/windows-validation.md)。
- 已生成基于 Inno Setup 的 EXE 测试安装器，带樱花图标、中文主要向导和
  当前用户启用/停用快捷方式；尚未在干净机器实际安装或卸载。

## 仓库结构

```text
Cargo.toml
Cargo.lock
build.rs
rust-toolchain.toml
README.md
LICENSE-MIT
LICENSE-APACHE
assets/
  flower.png
  flower.ico
  flower.rc
scripts/
  package.py
  test_package.py
  make_icon.ps1
  verify_windows_icons.ps1
  installer.py
  test_installer.py
installer/
  flower.iss
  notice.zh-CN.txt
  zh-CN.isl
src/
  bin/
    flower-register.rs
    flower-settings.rs
  lib.rs
  engine/
    mod.rs
    error.rs
    key.rs
    keymap.rs
    lexicon.rs
    router.rs
    session.rs
    types.rs
  platform/
    candidate.rs
    mod.rs
    composition.rs
    popup.rs
    registration.rs
    settings_ui.rs
    windows.rs
  registration.rs
  settings.rs
tests/
  candidate_popup.rs
  engine_router.rs
  pinyin_session.rs
  registration.rs
data/pinyin/mvp_lexicon.tsv
docs/distribution.md
docs/plans/
.github/workflows/windows-package.yml
```

分阶段实现与验收标准记录在
[`docs/plans/2026-10-20-flower-rust-implementation-roadmap.md`](docs/plans/2026-10-20-flower-rust-implementation-roadmap.md)。

## 公开 API

主要 API 位于四个模块中：

- `flower::engine`：按键、按键事件、键位映射、词库加载、组词、候选和
  会话错误。
- `flower::settings`：`Settings`、`SettingsStore`、`Theme` 和设置错误。
  `SettingsStore::for_current_user()` 在 Windows 上返回当前用户配置存储；其他
  平台明确返回不支持。
- `flower::platform`：候选弹窗快照与动作校验、组词宿主适配、平台支持、
  TSF 元数据、Windows COM 导出，以及 Windows-only 的非激活候选窗外壳。
- `flower::registration`：固定 Flower COM/TSF 身份、严格 `OsString` 命令行
  解析、x64 PE32+ DLL 校验，以及注册计划、回滚和用户启用操作的平台无关边界。

常见的引擎和设置项也会从 crate 根重新导出：

```rust
use flower::{keymap, EngineRouter, Key, KeyEvent, Modifiers};

let event = keymap::map_windows_virtual_key(b'A' as u32, Modifiers::SHIFT)
    .expect("supported key");
assert_eq!(event.key, Key::Char('a'));

let mut router = EngineRouter::new().expect("embedded MVP lexicon");
assert!(!router.is_composing());

let letter = KeyEvent::plain(Key::Char('n'));
assert!(router.test_key(letter).expect("key preview"));
assert!(!router.is_composing());

let update = router.process_key(letter).expect("key processing");
assert_eq!(update.preedit, "n");
```

在 Windows 上，可通过 `flower::platform` 和 crate 根访问
`FLOWER_TEXT_SERVICE_CLSID`、`DllGetClassObject` 和 `DllCanUnloadNow`。
`flower::platform::CandidatePopupWindow` 另行提供独立的候选窗呈现组件；
TSF 服务在激活线程创建和使用它，依赖宿主已有的消息循环，不启动独立 UI 线程。
窗口只接收候选快照和纯数据鼠标动作，不直接调用引擎或编辑文档。

## 构建与测试

从仓库根目录运行以下命令：

```text
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace
python -m unittest -v scripts/test_package.py
```

### Windows EXE 安装器

除 ZIP 外，现提供基于 Inno Setup 6 的离线测试版安装器。先准备已校验的 ZIP，
安装 Inno Setup 6，然后运行：

```text
python scripts/installer.py --package dist/flower-0.1.0-windows-x64.zip
python -m unittest -v scripts/test_installer.py
```

可用 `--iscc <ISCC.exe>` 指定编译器，`--output-dir` 选择未包含同名产物的目录。
生成 `flower-0.1.0-windows-x64-setup.exe`、`.exe.sha256` 和构建清单。
编译器只是开发依赖，目标机器不需要 Python、Rust 或 Inno Setup。

安装器需要管理员权限完成机器级注册；安装后各用户从开始菜单运行“启用
Flower（当前用户）”，再按 Win+Space 选择。它不自动启用、不改默认输入法，
不覆盖旧安装。当前输入直通故障仍未修复，安装器也尚未完成干净机器实装验收。
详见[分发说明](docs/distribution.md)及[安装器契约](design/windows-installer.md)。

安装 MSVC 目标后，可从根目录检查 Windows TSF 边界：

```text
cargo check --workspace --target x86_64-pc-windows-msvc
```

Windows 构建还需要 SDK 中的 `rc.exe`，用于把樱花图标嵌入 DLL 和两个 EXE。
常规构建直接使用仓库内的 ICO，不依赖外部图片路径。需要重建图标时参见
[`design/windows-branding.md`](design/windows-branding.md)。

在 Windows 上运行设置程序：

```text
cargo run --bin flower-settings
flower-settings --help
```

非 Windows 目标仍可运行 `flower-settings --help`；不带 `--help` 启动时会输出
中文“不支持”说明并以非零状态退出。

在非 Windows 目标上，`flower::platform::is_supported_platform()` 仍可使用
并返回 `false`；Windows COM 实现不会被编译。

## 注册、启用与注销

Flower 的 TSF 安装不是单纯写 HKCU。完整机器安装由三部分组成：64 位
`HKLM\Software\Classes\CLSID` 中的 COM 类、官方 TSF Profile Manager 中的
zh-CN 语言配置，以及 `ITfCategoryMgr` 中的键盘分类。用户是否在语言列表中
使用 Flower 是另一项当前用户设置。注册工具使用同一个固定 CLSID、语言配置
GUID 和描述，不会替换其他输入法，也不会把 Flower 设为全体用户默认输入法。
当前固定身份是 CLSID `{3D5B7CB1-96CB-4F44-A792-64304F6E7426}`、language
profile `{6C7782F4-5F59-42CF-8C29-F9A8D46F1B72}`、zh-CN LANGID `0x0804`
和显示名称 `Flower`。樱花图标以多尺寸资源嵌入 DLL 和两个 EXE，
TSF profile 使用 DLL 中的首个图标，不依赖外部 PNG 或 ICO。

先在 Windows 10/11 x64 上构建：

```text
cargo build --release --lib --target x86_64-pc-windows-msvc
cargo build --release --bin flower-register --target x86_64-pc-windows-msvc
cargo build --release --bin flower-settings --target x86_64-pc-windows-msvc
```

把 `flower.dll` 与 `flower-register.exe` 放在需要安装的位置。管理员打开
“以管理员身份运行”的终端并执行：

```text
flower-register register --dll C:\Path\To\flower.dll
```

省略 `--dll` 时，工具使用自身所在目录的 `flower.dll`，不依赖当前工作目录。
它会把指定的 `--dll` 解析为最终绝对路径，并拒绝非 AMD64 PE32+ DLL、缺少
DLL 文件标志的映像、带 NUL 的 UTF-16 路径、越界头/节数据，以及不符合 Flower
固定身份的既有 COM/TSF 数据。它不会调用 `LoadLibrary` 加载待注册的
`flower.dll`。注册路径保留 `canonicalize` 返回的原始 UTF-16 形式，不为了
显示而简化长路径或特殊路径语义。机器注册与注销使用系统级固定命名 mutex
串行化，避免多个管理员实例互相回滚对方刚写入的项。

注册完成后，每个需要使用的账户分别以普通用户身份执行：

```text
flower-register enable
```

然后按 Win+Space 手动选择 Flower。`enable` 成功表示当前用户的精确 Flower
profile 已交给 Windows 输入布局系统，不表示当前焦点应用已经切换到 Flower。
如果语言列表尚未刷新，可注销并重新登录。

停止使用某账户的 Flower：

```text
flower-register disable
```

管理员注销机器注册前，应让各用户先执行 `disable`：

```text
flower-register unregister
```

`unregister` 先移除 Flower 的 TSF 键盘分类和语言配置，最后才移除 COM 类；
TSF 清理失败时保留 COM 以支持修复后重试。COM 预检还要求
`InprocServer32` 没有未知子项；删除时先整键删除它，再整键删除根键，不逐值
擦除。根键删除失败时会尝试恢复已确认的 `InprocServer32`，失败则同时报告。
它不会删除 `flower.dll`、设置文件或其他用户的输入配置。重复注销、以及 DLL
文件已不存在时注销都可以安全执行。

常见限制：

- `register`/`unregister` 必须提升终端，工具不会自动弹 UAC 提权。`enable`/
  `disable` 不需要管理员权限。
- 机器注册不会为所有用户启用。其他用户需要自行运行 `enable`；注销也不会
  代替这些用户执行 `disable`。
- 已被宿主进程加载的 DLL 可能无法替换或删除。先运行 `disable`，退出正在使用
  Flower 的应用程序；若仍被占用，注销并重新登录或重启后再替换 DLL。
- 同一 CLSID 下出现不同 DLL 路径或额外注册数据时，工具会保守拒绝覆盖。
  确认旧数据确属 Flower 后，先备份对应注册表项，再只删除已经核实的条目并
  重新执行 `register`。不要用宽泛的树删除清理未知内容。
- `InstallLayoutOrTip` 返回 `FALSE` 时 Windows 未提供可靠原因，错误文本不会
  伪造 `GetLastError` 含义。请确认先完成机器注册、以目标用户运行 `enable`，
  并尝试重新登录后重试。

Windows 本机已生成并验证真实 ZIP/EXE 制品，并完成原始 DLL 的机器注册、
普通用户启用和 COM 查询。实际输入仍直通英文；注销重试、已加载 DLL、
多用户隔离和新 EXE 的安装/卸载等整体验收尚未完成。

## ZIP 分发

在 Windows 10/11 x64 上安装 Visual Studio C++ Build Tools、Windows SDK、
Rust stable 的 `x86_64-pc-windows-msvc` 目标、`rustfmt`、`clippy` 和
同工具链 `rust-docs` 后，从仓库任意当前目录运行：

```text
python C:\Path\To\flower\scripts\package.py
```

可加 `--output-dir` 指定输出位置。脚本不依赖当前工作目录，在非 Windows
平台 `--help` 可用，但实际组包会明确拒绝。输出为
`flower-<version>-windows-x64.zip` 和对应的 `.zip.sha256`；已有目标文件或
悬空符号链接不会被覆盖。ZIP 包含 DLL、两个 EXE、中文安装说明、词库、
`Cargo.lock`、项目与第三方许可证正文、`THIRD-PARTY-NOTICES.md`、
`manifest.json` 和覆盖全部 payload 的 `SHA256SUMS`，不包含 PDB、`.rlib`、
源码树或用户配置。

完整的构建依赖、SHA-256 校验、稳定目录安装、管理员 `register`、普通用户
`enable`、设置、停用/注销和升级步骤见
[`docs/distribution.md`](docs/distribution.md)。该 ZIP 未签名、不是 MSI；
包内词库改动不会热加载；CI 工作流尚未实际触发，Stage 2/3/4a/4b 的 Windows
桌面验收也仍未完成。

## 输入行为

- A-Z 会启动或扩展无音调全拼组词，并统一规范为小写。
- 数字 1-9 用于选择当前页上的候选。
- 空格或 Enter 会提交高亮候选；若不存在候选，则提交原始拼音输入。
- 退格键编辑组词内容，Escape 取消组词，PageUp/PageDown 切换候选页。
- 左/右方向键和上/下方向键移动高亮候选。
- Ctrl、Alt 和 Windows/meta 组合键会直通宿主程序。
- 默认每页显示 9 个候选，候选按词库权重排序。

## 词库

可复现的 MVP 词库是
[`data/pinyin/mvp_lexicon.tsv`](data/pinyin/mvp_lexicon.tsv)。该词库在编译期
嵌入，并使用三个以制表符分隔的 UTF-8 字段：

```text
lowercase-pinyin<TAB>candidate-text<TAB>weight
```

空行和以 `#` 开头的行会被忽略。权重为正的 `u32` 值，权重越大越靠前。
格式错误的数据行、非小写拼音键、无效 UTF-8、零权重和空词库会被
`EngineError` 拒绝。

## 设置

Windows 当前用户配置位于：

```text
%LOCALAPPDATA%\Flower\settings.json
```

JSON 包含 `candidate_page_size`（1-99）和 `theme`（`system`、`light` 或
`dark`）。路径通过 `SHGetKnownFolderPath(FOLDERID_LocalAppData)` 获取，不使用
进程工作目录或 DLL/EXE 所在目录。

`flower-settings` 使用原生 Win32 窗口、文本输入和主题下拉框，支持 Tab、Enter
和 Escape。保存会先校验，随后在配置目录创建唯一临时文件，完整写入并同步后再
用 Windows 原生替换/move 操作更新配置；失败时保留原文件并清理本次临时文件。
关闭窗口或选择“取消”不会保存。若已有配置损坏，设置程序会提示错误并让用户
明确选择使用默认值继续或退出，不会静默覆盖损坏文件。

保存成功后，需重新切换到 Flower 输入法使页大小和主题生效。TSF 激活时读取设
置失败会使用默认值并保持输入可用，不弹出宿主错误界面，也不改写损坏文件。

`system` 主题使用系统窗口、文字和高亮颜色，并响应系统颜色变化；它遵循 Windows
系统颜色及高对比度设置，不承诺自动追随 Windows 应用深色模式。`light` 和
`dark` 使用固定候选窗配色。

Windows 桌面手动验收仍需在真实 Windows 10/11 x64 环境执行：检查设置窗口的
Tab/Enter/Escape、取消不保存、保存后重开、损坏配置恢复提示，以及重新激活
Flower 后候选页大小和主题变化。交叉编译和 Linux 测试不能替代这些步骤。

Flower 不发起任何网络请求，不收集遥测数据，也不包含外部输入数据或模型。
