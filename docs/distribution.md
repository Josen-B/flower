# Flower Windows x64 ZIP 分发包

本文既是仓库中的分发说明，也会在成功组包后原样复制为 ZIP 包内的
`README.md`。Flower 的打包流程面向 Windows 10/11 x64 并生成未签名 ZIP，
不是 MSI 安装程序，也没有自动更新、自动提权、默认输入法替换或网络发布功能。

## 当前边界

- 仅支持 x64 Windows 宿主。x86 宿主、ARM64 宿主和其他平台尚未支持。
- ZIP 未做 Authenticode 签名。Windows 可能显示未知发布者提示；请先按本文
  校验下载文件和包内清单，但校验值本身不构成发布者身份证明。
- MVP 词库在编译时嵌入。包内 `data/pinyin/mvp_lexicon.tsv` 便于审计和再构建，
  修改它不会让已安装的 DLL 热加载新词；必须重新构建并替换二进制。
- TSF 使用机器级注册，输入法启用是每个当前用户单独操作。注册不会自动把
  Flower 设为所有用户的语言或默认输入法。
- 自动化检查不能代替真实 Windows 桌面验收。Stage 2、3、4a、4b 的桌面步骤
  以及本 Stage 4c 包的安装、注册、输入和升级流程仍待 Windows 10/11 x64
  实际验收。本文不把 Linux 上的构建或测试结果描述为 Windows 桌面已通过。
- 仓库中的 Windows 打包 CI 配置尚未实际触发或验证。真实 CI 结果、包内两个
  EXE 的 `--help` 运行和 Windows 产物验收仍待实际执行。

## 构建环境

在 Windows 10/11 x64 上准备：

- Python 3.11 或更新的 64 位解释器。Python 仅用于构建与测试，不是 Flower
  运行时依赖。
- Rust stable 工具链，包含 `rustfmt`、`clippy` 和 `rust-docs`。`rust-docs`
  必须来自当前 `rustc` 的同一工具链；打包器会从该工具链的 sysroot 读取
  Rust 标准库版权与许可证正文。
- Visual Studio 或 Build Tools 的“使用 C++ 的桌面开发”工作负载及 Windows
  SDK，以提供 MSVC 链接器和 Windows 导入库。
- Rust 的 `x86_64-pc-windows-msvc` 目标。

可先运行：

```powershell
rustup target add x86_64-pc-windows-msvc
rustup component add rustfmt clippy rust-docs
```

Rust MSVC 目标默认使用动态 C runtime。ZIP 不包含 VC++ Redistributable，也不会
自动下载或改成静态 CRT 链接。目标机器可能需要安装 Microsoft Visual C++
v14 x64 Redistributable；官方下载说明见
<https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist>。
不要假设只在干净的 Windows 系统上复制 ZIP 就一定具备所需运行库。

## 构建 ZIP

从仓库任意当前目录调用脚本均可；脚本按自身路径定位仓库：

```powershell
python D:\path\to\flower\scripts\package.py
```

默认输出到仓库根目录的 `dist\`。也可指定新的空目录或尚无目标产物的目录：

```powershell
python D:\path\to\flower\scripts\package.py --output-dir D:\Flower-output
```

`python scripts/package.py --help` 可在非 Windows 平台查看中文帮助并成功退出。
实际组包只支持 Windows；在 Linux、macOS 或其他平台不带 `--help` 运行会明确
拒绝并以非零状态退出。脚本没有跳过校验或使用测试 PE 的发布选项。

打包器使用 `--locked` 读取面向 Windows 的 Cargo 元数据并构建 release 的库和两个
可执行文件。若设置了 `CARGO_TARGET_DIR`，脚本使用 Cargo 元数据和本次 artifact
返回的实际绝对路径，不自行猜测 `target` 子目录。随后它会：

1. 只从本次 Cargo JSON artifact 中选择根包的 `flower.dll`、
   `flower-register.exe` 和 `flower-settings.exe`，拒绝缺失、重复、歧义或路径
   不存在的产物。root artifact 必须满足 release 契约
   `opt_level=3`、`debug_assertions=false`、`test=false`，且文件位于 Cargo
   元数据给出的 `target_directory/<target>/release`。
2. 严格检查三个文件都是 AMD64 PE32+，检查头、可选头、数据目录、节表和原始
   数据边界。DLL 必须带 DLL 标志并导出 `DllGetClassObject` 与
   `DllCanUnloadNow`；必需导出必须指向文件中有实际节数据支持的本地函数，
   不接受 forwarder。EXE 不得带 DLL 标志。此检查不加载或执行任何产物。
3. 从 Windows 目标解析后的 Cargo 依赖闭包收集本地许可证正文。缺少正文、未知
   SPDX 表达式或无法确认附加条款时中止，不联网补下载。
4. 从当前 `rustc` sysroot 的 `share/doc/rust` 收集
   `COPYRIGHT-library.html` 和许可证正文。若缺少该文档，请安装同工具链的
   `rust-docs`；不能用网络 master 文档替代。
5. 在输出目录内使用唯一临时 staging 完成全部校验、清单和 ZIP 写入，最后才
   发布 ZIP 与外部校验文件。已有 ZIP、校验文件或悬空符号链接时拒绝覆盖。

输出文件名为：

```text
flower-<version>-windows-x64.zip
flower-<version>-windows-x64.zip.sha256
```

ZIP 内只有一个顶层目录 `flower-<version>-windows-x64/`。白名单内容为：

```text
flower.dll
flower-settings.exe
flower-register.exe
README.md
LICENSE-MIT
LICENSE-APACHE
THIRD-PARTY-NOTICES.md
Cargo.lock
manifest.json
SHA256SUMS
data/pinyin/mvp_lexicon.tsv
licenses/
```

包内没有 PDB、Rust `.rlib`、源码树、`.git`、用户设置或候选历史。ZIP 使用固定
条目顺序、固定时间戳和固定权限，相同输入在同一打包实现下应产生相同字节；该
承诺不表示 Rust 编译输出本身可重复。

## 校验下载文件

先校验外层 `.zip.sha256`。该文件含 ZIP 文件名的 SHA-256。PowerShell 示例：

```powershell
$zip = "flower-0.1.0-windows-x64.zip"
$expected = ((Get-Content "$zip.sha256" -Raw) -split '\s+')[0].ToLowerInvariant()
$actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw "ZIP SHA-256 不匹配" }
```

解压后还可用包内 `SHA256SUMS` 校验每个 payload（该清单不包含自身）。如果环境
提供兼容的 `sha256sum`，可在解压后的顶层目录运行：

```powershell
sha256sum -c SHA256SUMS
```

也可以逐行读取 `SHA256SUMS` 并用 `Get-FileHash` 比较。不要只根据文件名或
版本号判断来源。

## 首次安装

1. 将 ZIP 解压到稳定的机器级目录，例如
   `C:\Program Files\Flower\0.1.0`。解压到 `Program Files` 通常需要管理员
   权限。之后不要从 `Downloads` 临时目录或会在重启时清理的位置注册 DLL。
2. 在解压后的目录中确认 `flower.dll`、`flower-register.exe` 和
   `flower-settings.exe` 来自同一个包，并先完成上述校验。
3. 让所有正在使用 Flower 的宿主程序退出。DLL 一旦被宿主加载，Windows 可能
   阻止替换、移动或删除。
4. 以管理员身份打开终端，执行机器级注册。建议使用 DLL 的完整路径：

   ```powershell
   & "C:\Program Files\Flower\0.1.0\flower-register.exe" register `
     --dll "C:\Program Files\Flower\0.1.0\flower.dll"
   ```

   省略 `--dll` 时，注册工具默认使用自身所在目录的 `flower.dll`。注册工具只
   校验并登记 DLL，不加载它，也不自动提权。
5. 每个希望使用 Flower 的用户分别以普通用户身份执行：

   ```powershell
   & "C:\Program Files\Flower\0.1.0\flower-register.exe" enable
   ```

6. 按 Win+Space 手动选择 Flower。`enable` 成功只表示当前用户的 Flower
   布局已启用，不表示当前焦点应用已经切换输入法。若语言列表尚未刷新，可
   注销并重新登录后重试。
7. 需要调整候选页大小或主题时，运行：

   ```powershell
   & "C:\Program Files\Flower\0.1.0\flower-settings.exe"
   ```

   配置保存在当前用户的 `%LOCALAPPDATA%\Flower\settings.json`。保存后重新
   切换到 Flower 才应用新设置。

## 停用与升级

停止某个用户使用：

```powershell
& "C:\Program Files\Flower\0.1.0\flower-register.exe" disable
```

管理员注销机器级服务前，应先让每个用户执行 `disable`：

```powershell
& "C:\Program Files\Flower\0.1.0\flower-register.exe" unregister
```

`disable` 和 `unregister` 不删除 DLL、用户设置或候选相关本地文件。升级到新
目录时，按以下顺序操作，避免同一 CLSID 指向两个目录中的 DLL：

1. 各普通用户运行 `disable`。
2. 退出所有可能加载 Flower 的宿主程序；若 DLL 仍被占用，注销并重新登录或
   重启后再继续。
3. 管理员运行 `unregister` 注销旧目录中的服务。
4. 把新版本解压到新的稳定目录并完成校验。
5. 管理员对新目录执行 `register --dll ...`，各用户再执行 `enable`，然后用
   Win+Space 选择 Flower。

直接覆盖已注册目录中的 DLL 可能遇到“文件正在使用”，也可能让新旧二进制、
配置和注册状态混用。升级时更换目录并先注销可让冲突更容易定位。任何时候都
不要用宽泛目录删除清理未知的注册表项或用户文件。

## 许可与第三方组件

Flower 自身按 `MIT OR Apache-2.0` 双许可发布，完整英文正文位于
`LICENSE-MIT` 与 `LICENSE-APACHE`。第三方 Rust 依赖和 Rust 标准库的来源、
SPDX 表达式、仓库地址及包内许可正文路径列在 `THIRD-PARTY-NOTICES.md`；
`licenses/` 中保留实际收集到的法律正文，包括 Unicode-3.0 等附加条款。SPDX
名称本身不能代替正文。

ZIP 清单 `manifest.json` 记录包名、版本、目标、三个二进制的角色/大小/SHA-256、
`rustc -Vv`、`Cargo.lock` 哈希，以及“未签名”和“Windows 桌面未验收”状态。
它不记录本机构建目录。包内 `SHA256SUMS` 覆盖除自身以外的全部 payload。
