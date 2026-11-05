# Flower Windows 验证记录

## 0.4.3：固定 Program Files、混输与宿主学习修复

以 `665ca3c5dc5cea62340bdba7aa7136ec01586f7e` 上未提交的 0.4.2 工作区为基线，
保留已有差异。最新契约为 `design/mixed-input-and-installation.md`。
用户最终取消自定义目录，EXE 固定 `{commonpf64}\Flower`，不是继续修复 D 盘自选安装。

### 实现及已有验证

- anywork 固定提交取证确认它采用非提升的当前用户普通应用安装；这不等价于机器 TSF。
  本版隐藏目录页并拒绝 `/DIR` 异根，继续严格目录安全检查，不修改已有 owner/ACL。
- 完成页默认勾选“启动 Flower”，仅在成功安装、注册及安全/hash 检查后以原始普通
  用户运行。严格启动参数先拒绝提升 token，再 enable 并打开设置；不改默认输入法。
  nowait 启动错误增加固定 MessageBox，保留 stderr 和失败退出。独立最新绑定补审批准；
  Python installer 22 项通过。此证据不等于实际完成页或消息框已展示。
- 混输采用共享规范音节 trie，保留真实 consumed、规范读音和本地调序作用域。
  每起点一次收集 prefix，整次 decode 共享 16384 状态预算，单查询最多 4096；
  exact/initial 独立保留。真实内置 `shenmeys` 首屏回归发现“意思”词边被 top4 裁掉，
  将既有词纳入人工日常层后通过，未造整句词条、改频率或扩 beam。
- 学习真实断点为应用范围读取：`GetProperty` 文档属性返回 E_FAIL，而
  `GetAppProperty` 同编辑会话内返回 S_OK 并调用 owner。原生 fixture 必须额外
  `ITfSource::AdviseSink(ITfContextOwner)`；已正确解除 sink。
  安全 0/57/58 通过生产按键、选择第二项“映射”、成功提交、激活内前移、
  下一次默认提交和 worker drain/隔离 Store 重开证明持久化。
  password/private/混合敏感/未知/空/VT_EMPTY/owner 失败可提交但不学习。
  新增固定诊断阶段仅 HRESULT，不记录输入。原生 3 项、诊断 2 项通过且独立审查批准。
- 首次集中检查库 63 项通过、5 项历史专用 I/O 忽略；settings CLI 3、
  adaptive 1、候选 23 通过。该命令随后因上述 mixed 首屏失败中止，不能描述为全套通过。
  返修后 common 3、句子 17、质量 5、decoder 预算 3、mixed 索引 2 项通过。
  未改 Store/进程持久化接口，复用 0.4.2 其有效证据；后续消费者检查另记。
- 最终引擎版本集中补验 settings 3、router 5、翻页 2、拼音会话 22、注册 25 全通过；
  `--lib native_` 实际命中 4 项通过（3 项 TSF + 1 项原生安全描述符）。
  严格全目标 Clippy 通过，旧 LNK4104 PRIVATE 导出链接告警仍在。
  格式检查发现新增 settings 断言及原生 fixture 排版，按 rustfmt 机械整理后通过，
  不改变行为；`git diff --check` 通过。

### 混输性能与独审返修

独审发现每个 prefix 重新启动 mixed 会重复使用 4096 配额，实际每键成本被放大；
已改为整次共享预算和每起点一次遍历，并用跨起点计数/强制耗尽测试验证。
进一步仅做等价优化：图首字母前四项在构造 Choice 前截取、每起点只克隆一次路径束、
满四边的结束位置提前跳过。新增等价测试及十组完整候选快照相同，不减少合法整词候选。

同机未修改的 `cargo run --release --example lexicon_probe` 固定负载包含126字节密集输入：
最初重复 mixed 查询逐键 P95 77.027 ms，预算修复后32.990 ms；
本轮局部优化前30.101 ms，优化后两次20.886/21.982 ms，
最大值25.950/42.751 ms有波动。两次P95比本轮前下降约27%–31%，不能声称最大值稳定下降。
历史0.4.0记录P95约8.313 ms，不能宣称新混输没有额外开销；
这些不是相对0.4.2的严格对照，也不是 TSF/绘制端到端时延保证。

最终新冻结混输审查批准，覆盖共享预算闭环、引号边界、完整身份、索引快照和等价优化。
安装启动与学习的有效限定审查仍保留，未因不相关排版或文案更新重复构建测试。

### 0.4.3 制品核验

真实 MSVC 和 Inno Setup 6.7.3 构建成功。安装器版本资源为 0.4.3，
签名状态 NotSigned，构建标识 `0.4.3-47f587e1fcae`。
版本字符串含 Inno 填充空白，核验取 Trim 后值；不是版本错误或重新构建。

| 文件 | 字节 | SHA-256 |
| --- | ---: | --- |
| `flower-0.4.3-windows-x64.zip` | 3,237,601 | `47f587e1fcaec3eae436f228a387f967114fb2c7c7ed3e5b5b06cadedc944881` |
| `flower-0.4.3-windows-x64-setup.exe` | 3,392,741 | `1a6072d250a8e2dbd7e9f50d38ba0426d044136ce070aef53c993b2a3167cc3d` |

85 个 payload（含 SHA256SUMS）、全部内外哈希、源 ZIP 关联、完整许可、
README 快照、DLL 精确嵌入 base/core 且不含 optional、两个 help、
Unicode 输出/非法标量拒绝、三个 PE 共 27 张图标均核验。
help 包含启动入口；非法附加参数拒绝，未运行有效启动命令。
机器路径 helper 只读接受本机 Program Files 下新目录并拒绝卷根/网络/Windows 系统树，
未创建目标；它不证明 EXE 接受自选目录。固定目录/异根拒绝由模板测试及审查覆盖。
旧 0.3.0、0.4.0、0.4.1、0.4.2 ZIP/EXE 哈希保持不变；
新包及两个校验文件、安装器 manifest 以不覆盖方式复制到 `dist/`。

### 未覆盖

不运行真实安装、prepare/register、有效启动入口或真实用户词典命令；不关闭应用、
注销、改默认输入法、已有 ACL 或 Git 状态。真实安装/卸载、另一账户提权、
用户现场“映射”排名、实际混输与候选窗口、多屏/DPI 仍待复测。
完全不声明 scope 的普通宿主仍不学习，不能宣称已解决所有应用中的前移故障。
签名发行和远端 CI 未验收；本版仍为离线未签名测试包。

## 0.4.2：完整所选目录、严格单行与持久相邻调序

基线 `665ca3c5dc5cea62340bdba7aa7136ec01586f7e`。用户明确选择：
全部程序放所选目录、保留安全限制且不改已有父权限；选词顺序本地保存并跨重启保留。
权威契约见 `design/adaptive-candidates.md`，本轮仍只递增补丁版本。

### 实现

- 安装器恢复真实目录页，载荷、注册、公共入口、卸载器均使用所选 `{app}` 根，
  无另一份固定 Program Files 载荷，不再部署用户副本。默认 Program Files 不等于强制。
  父目录不安全仍拒绝，未以目录深度为理由；系统注册表/快捷方式/用户配置不是程序载荷。
- 宽预算与候选数量共同确定连续 `page_ranges`，数字键、鼠标、Enter、页码同源；
  单行宽封顶560逻辑像素、默认内容预算544，最多九项，长词减少数量，超长单项独页。
  单项估算由 engine 与布局共享，所有候选可翻页，不隐藏尾部或截断实际提交。
- 保存显式候选顺序而非增加512权重；完整候选在成功安全提交后仅与直接前驱交换，
  第一位不动，部分选择/试探/失败/失焦/敏感及未知范围不提升。激活内下一次新输入
  即时同步，正在组词的列表不偷偷换位置；一次性claim防重复完成推进。
- schema2增加默认true的自适应开关及顺序字段，schema1只读迁移、不在load时重写，
  下次合法更新才写schema2。旧计数保留但不再作为TSF权重叠加。
  关/清空、generation、防过期批次、有界队列、锁内最新状态合并与原子发布均保留。
  设置窗口与CLI提供关闭/清空，旧二进制不识别schema2，降级须备份。

### 实际验证与返修

首次 `cargo test --locked --all-targets`：库56通过、5个历史专用I/O测试忽略，CLI3通过；
随后候选几何一项失败。原因是测试用9项各61像素（总549）断言不应压缩，但内容预算544。
修正为8项验证精确估算、9项验证整页压缩，并改进不足预算时的整数余数分配。
修复后候选23项通过，未删除候选或放宽无遗漏断言。

后续分批执行并复用未受影响证据：

- 映射/Store/新会话集成1项通过：明确第二位fixture选择“映射”，保存后用新Store加载，
  在真实内置词典的新会话中成为首项并可提交；不依赖上游频率恰好将它列第二。
- 常用排序2、router5、质量5、翻页2、句子15、拼音22、注册25全部通过。
  含第五项只前移一位、交错顺序、预算变化保留绝对高亮、真实页范围无遗漏、
  局部索引与只读预演、完整组合身份及partial不记无解后缀。
- 用户词典30通过、1个显式子进程helper忽略；进程套件5通过、1个专用helper忽略。
  覆盖schema1不落盘迁移、schema2重放、开关/清空/代次、非法批次原样保留、
  两Store与独立进程最新顺序合并、占用/只读/异常退出及原子失败保护。
  进程组中的辅助测试由实际并发回归显式启动。
- Python installer21项通过；不重复运行未改的数据转换与package单元套件。
- 严格全目标Clippy、格式检查通过。旧LNK4104 PRIVATE导出链接告警仍在。

独立审查发现发布前宽预算可能使用旧显示器DPI。修正为同屏读实时窗口DPI，
跨屏创建独立隐藏STATIC探针，验证monitor及awareness context相同后读目标DPI，
失败默认预算，不用旧屏值伪装成功。所有路径销毁，不移动实际popup、
不改宿主或线程DPI awareness，不在OnTest预演创建资源。
目标96/192DPI预算定向测试1项通过，生产路径新冻结补审批准。
持久化及TSF消费者独立静态审查通过；已有其他有效审查范围未因局部返修清零。
以上不是一次全套命令全过，也不是多屏或真实宿主验收。

### 0.4.2 制品

真实MSVC和Inno Setup6.7.3构建成功，版本0.4.2，仍为NotSigned。

| 文件 | 字节 | SHA-256 |
| --- | ---: | --- |
| `flower-0.4.2-windows-x64.zip` | 3,202,259 | `479d2998d9492dcabc987a1c47ef37cb25b352d5f45bd3de441704495958112a` |
| `flower-0.4.2-windows-x64-setup.exe` | 3,369,102 | `ac170c5aae6163b1ab743e313f363fb33c71a0c5b180a63a217adc76775a57a7` |

85个payload（含SHA256SUMS）、包内外哈希、源ZIP关联、完整许可、README快照、
DLL精确内嵌本次base/core且不嵌optional、两个help、Unicode输出/非法拒绝及三个PE的
27张图标均核验。旧0.3.0、0.4.0、0.4.1 ZIP/EXE哈希未变。
新包真实机器路径只读检查接受本机D盘根下及Program Files下的新目录；本机
不安全D:\software祖先仍拒绝，检查未创建目录、未改权限。

### 未覆盖与升级

未运行真实安装器、prepare/register、真实用户词典命令，未改默认输入法、已有ACL、
关闭应用或注销；未Git add/commit/reset。完整安装/卸载、另一台机器输入scope与本地调序、
真实多屏/DPI及Edge等宿主仍需用户复测，远端CI与签名发行门禁仍未通过。
升级前正常卸载0.4.1并释放旧宿主；旧用户副本/未知残留不由本版擅自删除。

## 0.4.1：深层目录、横排候选与输入兼容改进

用户确认 0.4.0 可在 `D:\Flower` 安装，但已有 `D:\software` 下拒绝；记事本显示
候选，Edge/其他软件未显示。此轮只递增补丁版本 0.4.1；以后无明确要求不升级大版本。
基线 `d6839a76b3d48b3bb07b56c739e955b8bad5d6a7`，未执行 Git 写操作。

### 改动与决策

- 深层路径不是层数限制：已有父目录可能由普通用户拥有，并允许删除/替换。
  用户将推荐方案交由根选择，采用分组件布局。DLL、注册工具、卸载器及公共入口
  固定在 Program Files 的安全位置；用户文件目录可选正常本地盘深层路径。
  用户副本由 `ExecAsOriginalUser` 调用经核对的系统 helper 部署，提升令牌拒绝，
  无管理员写入用户可替换目录的兜底，不修改已有父 ACL/owner。
- `+`/`=`/小键盘加号下一页，`-`/小键盘减号上一页；Shift+- 不吞下划线，
  Ctrl/Alt/Win 和非组词直通。保留既有 PageUp/PageDown 和候选导航。
- 候选按行从左至右排列、空间不足换行；不绘制拼音头部及读音注释，文档预编辑仍在。
  移除 32 项静默截断，覆盖页大小 1–99；极小空间整页退化，键盘操作继续有效。
- 布局查询失败/无效时尝试与 view 关联的 GUI caret，再以有效 viewport 边缘作粗锚；
  clipped 与无效视图仍隐藏。仅在编辑会话查询，保留焦点、激活、revision 门禁。
  显示提升普通 Z 序，不设永久置顶、不抢焦点。这是已知缺口修正，未复现用户 Edge 现场。
- 用户选择日常词整体优化：135 个项目维护词中 132 个匹配已有真实词条，使用独立
  日常优先层，不改数据 weight、不造读音。全拼、整词简拼、组合、部分前缀依次显示；
  真实内置 `nh/wm/sm/ky/zg/xx` 的目标词均在默认九项首屏并可正确提交。
  该层不是语料频率，不是无偏准确率基准；同层仍按原词频稳定排序。

### 自动验证

- 集中 Rust 150 项通过：库54（含几何、SDDL及原生TSF编辑），CLI3、候选22、
  常用排序2、router5、质量5、翻页2、句子15、拼音17、注册25。
- 后续将固定用户文件复制抽为私有函数，真实入口仍保留非提升、可信源与目标 guard。
  显式运行默认忽略的五项隔离 I/O 检查：深层创建及内容一致、拒绝覆盖、后续源缺失
  回滚、文件竞争、目录竞争，5 passed（0.35 s）；重解析属性纯内存检查另1项通过。
  仅操作当前 Thread scratch 下自有 fixture，假 EXE 不执行，提升进程会在 I/O 前失败。
- 严格全目标 Clippy、格式检查通过。Python installer 21 项通过；未改数据转换或
  package 逻辑，不重复执行无关全套。旧 LNK4104 PRIVATE 导出链接告警仍在。
- 横排/翻页/定位独立静态审查通过；分组件初版审查与最终私有复制增量补审通过。
  旧冻结批准没有外推为新版本补验通过。

主要入口：

```powershell
cargo test --locked --lib --bin flower-register --bin flower-settings --test candidate_popup --test common_ranking --test engine_router --test lexicon_quality --test pinyin_sentence --test pinyin_session --test paging_keys --test registration
# FLOWER_INSTALL_PATH_TEST_SCRATCH 须指向当前 Thread 的 scratch
cargo test --locked --lib isolated_user_copy_ -- --ignored
cargo test --locked --lib user_directory_reparse_attributes_are_rejected_without_creating_links
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
python -m unittest scripts.test_installer -v
```

### 0.4.1 制品核验

真实 MSVC、Inno Setup 6.7.3 构建成功，版本资源 0.4.1，仍为 NotSigned。

| 文件 | 字节 | SHA-256 |
| --- | ---: | --- |
| `flower-0.4.1-windows-x64.zip` | 3,176,172 | `e787702afcb771eb46af576c2a0f9c387b43097e265d696510779c0bd1523209` |
| `flower-0.4.1-windows-x64-setup.exe` | 3,355,568 | `f380a62c4624a1913c4b8125284337f29dfd92b9cb13a58eefb7ed1a77287b42` |

85 个 payload（含 SHA256SUMS）、内外哈希、源ZIP关联、许可、README快照、
DLL内嵌本次base/core且不嵌optional、两个help、Unicode输出与非法标量拒绝、
三个PE共27张图标均核验。旧0.3.0、0.4.0 ZIP/EXE哈希保持不变。

真实新包的 `check-user-install-path` 接受
`D:\software\Flower-0.4.1-readonly-check\deep\more`，检查前后目标均不存在。
机器路径检查仍正确拒绝卷根、UNC和Windows系统树。
未运行安装器、prepare、register或真实用户词典命令。

### 尚未覆盖

另一台机器的完整安装/卸载、另一账户提升的原始用户部署、实际 Edge 和其他宿主的候选
显示、字体/主题/DPI、真实 scope/学习与远端 CI 均未验收；分组件方案的系统DLL实际位于
Program Files，不能将深层用户副本目录描述为DLL安装位置。没有关闭用户应用、注销、
改默认输入法、已有ACL、注册或真实用户数据。保留后续实机反馈门槛，不宣称正式发行通过。

## 以下为 0.4.0 历史记录

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
