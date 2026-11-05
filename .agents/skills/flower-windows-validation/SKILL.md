---
name: flower-windows-validation
description: 在 Flower 仓库执行 Windows x64 原生构建、ZIP 验证、COM/TSF 安装和桌面排障时使用；区分可执行检查、当前用户启用和实际输入证据。
---

# Flower Windows 验证

先读 `docs/distribution.md`、`docs/windows-validation.md` 和当前 `design/mixed-input-and-installation.md`，分页与调序见 `design/adaptive-candidates.md`，旧版补充见 `design/input-and-installation.md`。历史设计文件若已不存在，不按旧摘要恢复；以当前源码和实际宿主事实为准，不将历史通过记录当成本次版本通过。

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

## 新建自定义安装目录

- 当前目录策略契约见 `design/input-and-installation.md`。不要把缺失目标的最近已有祖先当作安装根严格检查；允许无关兄弟创建/属性写入，但拒绝普通目录删除、子项删除及改 DACL/owner。已验证的本地固定卷根不可像普通目录删除，非严格祖先检查仅可忽略根本身 DELETE，不忽略 FILE_DELETE_CHILD 或权限接管；不把此例外应用到子目录。
- `check-install-path` 始终只读；`prepare-install-path` 是显式管理员写操作。后者仅对缺失组件用创建时的受保护 SDDL，新建即可信 owner/管理员SYSTEM全控/Users RX，保留禁止删除共享句柄后复查；不先创建可写目录再收紧，不修复已有对象，不自动改 D 盘根或父目录 ACL。同名竞争失败，不递归清理未知内容。
- 安装器必须在旧注册/目录冲突及运行库门禁后、复制前 prepare/check。只读新路径检查通过不代表管理员创建/向导安装成功；完整许可和源ZIP核验也不等于运行期安装验收。新制品仍不得自动运行。

## 0.4.3 固定安装、启动与真实学习链路

- 最新用户决定取消自选目录，EXE 固定 `{commonpf64}\Flower`、隐藏目录页、拒绝 `/DIR` 异根。下文自选目录章节仅为历史/helper能力，不是当前安装器承诺。不修改已有父目录权限或 owner，不把普通应用的 HKCU 安装方案当成已证实的 per-user TSF 注册。
- 完成页默认启动用 `postinstall nowait skipifsilent runasoriginaluser`，不加 `unchecked`；先检查安装/注册成功、安全树和设置程序哈希。启动入口严格解析，提升 token 或查询失败均拒绝；普通用户 enable 后打开设置，不改默认输入法、不声称焦点已切换。nowait GUI 错误必须有固定 MessageBox，不能只有 stderr。包验收不能运行该入口，因为会修改用户启用状态。
- `GUID_PROP_INPUTSCOPE` 应用声明读取使用同一编辑会话的 `GetAppProperty`；`GetProperty` 的文档属性不等价。未知、空、失败及敏感仍拒绝学习，不能为修复正向通路放宽门禁。
- 原生 ITfContextOwner 测试除 CreateContext 外，还须对 context 的 ITfSource 显式 AdviseSink，并在析构解除。用生产按键→成功提交→安全发布→Activation 排名→worker drain→隔离 Store 重载串起前移证据；仅 Store fixture 或 adaptive=false 测试不证明学习生产链。测试目录必须显式隔离，不用真实用户数据。

## 0.4.2 安装与严格单行（历史安装策略）

- 用户明确选择全部载荷放所选目录并保留安全限制；安装器恢复真实 `{app}` 根、版本载荷、注册/快捷方式/卸载器均同根，不再固定 Program Files 第二份，不调用用户副本部署。已有不安全祖先仍拒绝，不将深度当原因，也不改其 owner/ACL。Windows 注册表、快捷方式和用户配置不属于可搬迁程序载荷。
- 严格单行必须与 engine 真分页一起改。session page_ranges 是所有数字、鼠标、Enter、页码真源，不能 popup 少画尾项。共享 engine::paging::candidate_cell_width，560逻辑像素总宽、默认544内容宽，最多min(设置上限,9)；超长项独页省略但提交不截断。预算变化保留绝对高亮并发布新revision，旧票据失效。
- 跨屏不能在发布前拿旧 popup DPI 换算目标工作区。同屏可读取实时DPI；跨屏用无owner/parent、不显示、不激活的内置STATIC隐藏探针，先核对目标monitor与awareness context相同再GetDpiForWindow，所有路径销毁；失败默认预算并由单行布局缩窄。不移动已有popup、不改宿主或线程awareness，不在OnTestKey预演创建资源。
- GetDpiForMonitor 有调用方 awareness 限制，GetScaleFactorForMonitor 也不能未经核实当作精确窗口DPI。不要只替换API名称就宣称修复；纯96/192预算测试应接入真实生产查询链，真实多屏仍另验收。

## 0.4.1 分组件安装与候选窗（历史）

- 深层用户目录与机器 DLL 分离：系统载荷、公共入口及卸载器固定 commonpf64 Flower；自选目录仅固定用户文件副本。不得把关闭祖先 ACL 门禁或改已有父 owner 当作深路径支持。向导明确两种位置，ExecAsOriginalUser 从已核对安全载荷运行 helper，helper 查询 token 失败/提升均拒绝，不回退提升复制。
- install_user_files 的非提升检查、严格 source 检查、目标 guard 不能被 private copy helper 抽取绕过。固定文件 create_new、独占句柄、sync，失败仅通过本次持有句柄标记删除。源/目标 guard 覆盖复制及回滚，卸载不递归清理可替换用户树。
- `isolated_user_copy_` 五项测试默认忽略，必须使用本 Thread 的 `FLOWER_INSTALL_PATH_TEST_SCRATCH` 显式运行 `cargo test --lib isolated_user_copy_ -- --ignored`。只操作独占新建 fixture；提升进程在 I/O 前应明确失败，不以空 return 假通过。假 EXE 只当数据，不执行，不需要真实安装。
- GetTextExt 无布局、clipped 和无效 RECT 是不同情况；有效 caret 可零宽但须正高度。编辑会话内优先文字位置，其次关联 view HWND 的 GUI caret，最后可用有效 viewport 边缘作粗锚，不能称精确 caret。焦点/token/revision 门禁不放宽；popup 普通 Z 序提升且 NOACTIVATE，非 TOPMOST。
- 横向候选布局与绘制/命中同源，不画拼音头部或 comment；页大小可达99，不能沿用32项截断。极小区域整页退化无鼠标区域，保留键盘语义。截图/隔离几何测试均不替代 Edge 实机证据。

## 用户词典发布前进程终止回归

- 可用隔离 `DictionaryStore::new` 指向本 Thread scratch 下词典，父进程持有目标的无 delete-sharing 只读句柄，让真实生产写入停留在原子发布前；不得使用真实用户词典或填满工作磁盘。
- 观察唯一临时文件，并以拒绝 WRITE sharing 的只读打开确认生产写句柄已关闭；结合生产路径 `write_all -> sync_all -> close -> publish` 确定检查点，不用固定 sleep 宣称已进入写入阶段。
- 用测试程序自身的精确 helper 启动子进程，只终止该测试拥有的 child 并 wait/reap；断言失败也须通过 guard 停止并回收子进程。读取 staged JSON 检查新 generation，再强制结束，验证旧目标逐字节不变、可重新加载与管理写入、orphan 不被误恢复。清理仅限本次测试目录。
- `tests/user_dictionary_process.rs` 的 `killed_writer_before_atomic_publish_preserves_old_bytes_and_allows_recovery` 是定向入口。此类证据只覆盖同步完成但尚未发布时的进程结束，不证明写入中途、真实磁盘满、系统崩溃或掉电耐久性；不要为扩大表述而自动执行危险注入。

## 交接

记录源码版本/差异、实际命令、产物哈希、安装绑定、当前用户启用状态、桌面观察及未覆盖项。未签名包或失败的桌面验收不得标为可发布。旧归档保持不可变，并说明其是否包含后续源码修改。日志和探针放当前会话目录，不把会话绝对路径或用户输入沉淀到长期文档/技能。
