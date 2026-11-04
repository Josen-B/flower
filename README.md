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
- 完整上下文焦点处理、TSF 配置文件注册、安装打包以及真实 Windows 主机冒烟
  测试仍属于后续工作。当前 Linux 工作区不能替代 Windows 桌面交互验证。

## 仓库结构

```text
Cargo.toml
Cargo.lock
rust-toolchain.toml
README.md
src/
  bin/
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
    settings_ui.rs
    windows.rs
  settings.rs
tests/
  candidate_popup.rs
  engine_router.rs
  pinyin_session.rs
data/pinyin/mvp_lexicon.tsv
docs/plans/
```

分阶段实现与验收标准记录在
[`docs/plans/2026-10-20-flower-rust-implementation-roadmap.md`](docs/plans/2026-10-20-flower-rust-implementation-roadmap.md)。

## 公开 API

主要 API 位于三个模块中：

- `flower::engine`：按键、按键事件、键位映射、词库加载、组词、候选和
  会话错误。
- `flower::settings`：`Settings`、`SettingsStore`、`Theme` 和设置错误。
  `SettingsStore::for_current_user()` 在 Windows 上返回当前用户配置存储；其他
  平台明确返回不支持。
- `flower::platform`：候选弹窗快照与动作校验、组词宿主适配、平台支持、
  TSF 元数据、Windows COM 导出，以及 Windows-only 的非激活候选窗外壳。

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
```

安装 MSVC 目标后，可从根目录检查 Windows TSF 边界：

```text
cargo check --workspace --target x86_64-pc-windows-msvc
```

在 Windows 上运行设置程序：

```text
cargo run --bin flower-settings
flower-settings --help
```

非 Windows 目标仍可运行 `flower-settings --help`；不带 `--help` 启动时会输出
中文“不支持”说明并以非零状态退出。

在非 Windows 目标上，`flower::platform::is_supported_platform()` 仍可使用
并返回 `false`；Windows COM 实现不会被编译。

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
