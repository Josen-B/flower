---
name: flower-windows-validation
description: 在 Flower 仓库执行 Windows x64 原生构建、ZIP 验证、COM/TSF 安装和桌面排障时使用；区分可执行检查、当前用户启用和实际输入证据。
---

# Flower Windows 验证

先读 `docs/distribution.md`、`docs/windows-validation.md` 和 `design/windows-diagnostics.md`，以当前源码和实际宿主事实为准，不将历史通过记录当成本次版本通过。

## 环境与构建

- Windows MSVC 目标需要 Rust、MSVC 工具和 Windows SDK，不需要完整 Visual Studio IDE。先检查现有工具及 `vswhere`，不要重复安装整套工作负载。
- 安装系统依赖须有用户授权；UAC/安装取消不是成功，核对实际文件及注册状态，不盲目重复提权。
- PowerShell 中每次 exec 独立；需要时为该命令补 Cargo PATH，并使用已确认的 Python 路径，避免 WindowsApps 别名。
- 运行适用范围的 Rust 检查和 Python 组包测试。`cargo check` 不能代替原生链接和测试，合成 PE 测试不能作为可运行交付。
- 实际组包使用 `scripts/package.py`，保留仓库正常 Cargo 输出入口。新输出目录使用本 Thread 最新 artifacts 事实，避免覆盖已有 ZIP。

## 包与安装

- 校验外层 `.zip.sha256`、单顶层结构、包内 `SHA256SUMS`、manifest、三个真实 PE/导出、许可和两个 EXE 的 `--help`。
- 图标由 `build.rs` 调用 Windows SDK `rc.exe` 嵌入。运行 `scripts/verify_windows_icons.ps1 -BinaryRoot <解压目录>`，以资源方式加载三个 PE，对照 `assets/flower.ico` 检查组图标和各尺寸图像；不得执行 DLL 导出来验证资源。
- 品牌名称更新保持 CLSID/profile GUID 不变。旧名称安装应使用旧稳定目录的注册工具先注销，不通过放宽所有权检查或直接覆盖 registry 来改名。
- 机器 `register`/`unregister` 要管理员；普通用户 `enable`/`disable` 只作用于目标当前用户。不要更改默认输入法、重写语言列表或清理其他输入法。
- 从稳定安装目录注册，不从 session scratch、下载目录或 target 注册。冲突时停止，不覆盖未知安装。
- 启用成功不证明已切换焦点；COM 实例创建和 QI 成功不证明 `Activate`、按键接收器或编辑会话成功。
- `Get-WinUserLanguageList` 在 Windows PowerShell 中可返回集合对象；用 `foreach ($lang in (Get-WinUserLanguageList))` 访问成员，避免误把管道投影的 null 当作缺少语言配置。

## EXE 安装器

- `scripts/installer.py` 使用 Inno Setup 6 包装已校验的 ZIP，输出 EXE、外部校验文件和不含本机路径的构建清单。可用 `--iscc` 指定编译器；安装目录可能由 Windows 已知文件夹重定向，优先实际卸载记录，不猜工具路径。
- 新增安装器后运行 `scripts/test_installer.py` 的边界测试，并真实编译。编译后只读检查版本资源、图标、签名状态和交付哈希，不把编译成功当成实际安装或卸载通过。
- 当前测试安装器保守拒绝已有注册，不做自动升级。提升进程只做机器注册；用户启用/停用由普通权限开始菜单入口执行。不要把管理员账户当作目标用户。
- 注册失败需保留可能被 COM/TSF 引用的文件并返回失败；卸载需校验文件、路径所有权和占用，失败保留文件。干净虚拟机上的成功、失败和占用验收是独立门槛。
- 新制品校验应保留原 ZIP 的哈希关联与 Inno Setup 引擎许可，不覆写已经交付的旧 ZIP，也不为测试安装器自动改变当前桌面输入法。

## 桌面排障

- 必须记录用户实际宿主和具体现象：英文直通、吞键、已有预编辑但不能提交、卡住/崩溃，不能仅凭“无法输入”推断故障位置。
- 输入法不显示时先只读检查机器 profile、HKCU Enable、排序和用户语言列表；登录会话刷新是用户操作，不自动注销或重启。
- 对照记事本与 Chromium/Electron 宿主，并确认对应进程实际加载的 DLL 路径；不向用户现有文档自动发送按键。
- 切换 DLL 路径前停用目标用户、释放旧宿主并注销旧路径；窗口关闭不等于进程退出。只读检查已加载模块，不强杀用户程序或 Explorer。
- 默认关闭的调试构建可以在构建时显式设置 `FLOWER_TSF_DIAGNOSTIC_FILE` 绝对路径。release 不写日志。仅记录固定阶段、pid、HRESULT，不记录输入或文档内容。
- 独立 TSF 探针必须区分 host client、service client 和已存在接收器。客户端标识无效或重复 advise 的错误不等于真实宿主故障，不能据此改成异步或强行吞键。

## 隔离原生回归与目录安全

- 可在 STA 中创建真实 ThreadMgr、DocumentMgr 和无外部 text store 的 Context，直接调用服务按键回调，再通过真实编辑会话读回新建文档。测试不得全局注入按键或触碰用户文档。绕过 Activate/Advise 的测试只证明编辑路径，不证明真实宿主激活。
- StartComposition 必须传入生产使用的非空 composition sink；验证预编辑、提交后光标顺序、Escape 和终止后的干净恢复。模拟宿主强制终止用 ITfContextOwnerCompositionServices::TerminateComposition；发起方自己的 EndComposition 不保证回调同一个终止 sink。
- 自定义机器安装目录不能仅开放向导路径页。用 check-install-path 只读检查本地固定盘、重解析点、owner/DACL、祖先替换权限及实际 payload 树；不要通过增加 users-readexec ACE 假装移除了继承写权限，也不要擅自改 D 盘父目录 ACL。
- 安装前后及卸载前的 helper 必须来自核对哈希的私有临时副本；在执行安装树内 helper 前完成 ACL 检查。原生 SDDL 内存描述符可验证 allow/deny、inherit-only、null DACL 和未知 ACE，不需修改系统 ACL。SDDL 的 DC 是目录服务权限，不是文件系统 FILE_DELETE_CHILD；后者应使用 0x40。

## 交接

记录源码版本/差异、实际命令、产物哈希、安装绑定、当前用户启用状态、桌面观察及未覆盖项。未签名包或失败的桌面验收不得标为可发布。旧归档保持不可变，并说明其是否包含后续源码修改。日志和探针放当前会话目录，不把会话绝对路径或用户输入沉淀到长期文档/技能。
