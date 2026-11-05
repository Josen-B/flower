use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::ExitCode;

use flower::user_dictionary::DictionaryStore;

const USAGE: &str = "Flower 设置程序

用法：
  flower-settings
  flower-settings --help
  flower-settings --start-input-method
  flower-settings dictionary status
  flower-settings dictionary import <path> [--domain <name>]
  flower-settings dictionary export <path> [--domain <name>]
  flower-settings dictionary remove <canonical-pinyin> <text> [--domain <name>]
  flower-settings dictionary domain <name> on|off
  flower-settings dictionary learning on|off
  flower-settings dictionary clear-learning
  flower-settings unicode U+<hex>

无参数打开设置窗口。词典管理仅操作当前用户本地数据，不提权。
导入/导出使用三列 TSV（拼音、文字、正整数权重）；读音以单引号分音。
选词顺序默认本地保存；成功选中前移一位。可关闭或清空，不删除个人词条。
修改后请重新切换到 Flower 输入法使设置生效。";

#[derive(Debug, PartialEq)]
enum Command {
    Gui,
    StartInputMethod,
    Help,
    Unicode(char),
    Status,
    Import(PathBuf, Option<String>),
    Export(PathBuf, Option<String>),
    Remove(String, String, Option<String>),
    Domain(String, bool),
    Learning(bool),
    ClearLearning,
}

fn text(value: &OsStr) -> Result<String, String> {
    value
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| "命令、词语和域名必须是有效 Unicode。".to_owned())
}

fn switch(value: &OsStr) -> Result<bool, String> {
    match value.to_str() {
        Some("on") => Ok(true),
        Some("off") => Ok(false),
        _ => Err("开关参数只能是 on 或 off。".to_owned()),
    }
}

fn domain(args: &[OsString]) -> Result<Option<String>, String> {
    match args {
        [] => Ok(None),
        [flag, name] if flag == "--domain" => {
            let name = text(name)?;
            if name.is_empty() {
                return Err("域名不能为空。".to_owned());
            }
            Ok(Some(name))
        }
        _ => Err("可选参数必须为 --domain <name>。".to_owned()),
    }
}

fn unicode_scalar(value: &str) -> Result<char, String> {
    let invalid = || "码点必须为 U+<十六进制>，且为非控制字符的 Unicode 标量。".to_owned();
    let hex = value.strip_prefix("U+").ok_or_else(invalid)?;
    if hex.is_empty() || hex.len() > 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let scalar = u32::from_str_radix(hex, 16)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(invalid)?;
    if scalar.is_control() {
        return Err(invalid());
    }
    Ok(scalar)
}

fn parse(args: &[OsString]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Gui),
        [arg] if arg == "--start-input-method" => Ok(Command::StartInputMethod),
        [arg] if arg == "--help" || arg == "-h" => Ok(Command::Help),
        [cmd, value] if cmd == "unicode" => Ok(Command::Unicode(unicode_scalar(&text(value)?)?)),
        [cmd, rest @ ..] if cmd == "dictionary" => match rest {
            [op] if op == "status" => Ok(Command::Status),
            [op] if op == "clear-learning" => Ok(Command::ClearLearning),
            [op, value] if op == "learning" => Ok(Command::Learning(switch(value)?)),
            [op, name, value] if op == "domain" => {
                let name = text(name)?;
                if name.is_empty() {
                    return Err("域名不能为空。".to_owned());
                }
                Ok(Command::Domain(name, switch(value)?))
            }
            [op, path, tail @ ..] if op == "import" || op == "export" => {
                if path.is_empty() {
                    return Err("文件路径不能为空。".to_owned());
                }
                let name = domain(tail)?;
                if op == "import" {
                    Ok(Command::Import(PathBuf::from(path), name))
                } else {
                    Ok(Command::Export(PathBuf::from(path), name))
                }
            }
            [op, pinyin, word, tail @ ..] if op == "remove" => {
                Ok(Command::Remove(text(pinyin)?, text(word)?, domain(tail)?))
            }
            _ => Err("词典子命令或参数无效。".to_owned()),
        },
        _ => Err("不支持的命令或参数。".to_owned()),
    }
}

#[cfg(target_os = "windows")]
fn require_non_elevated() -> Result<(), String> {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| e.to_string())?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        let _ = CloseHandle(token);
        result.map_err(|e| e.to_string())?;
        if returned != std::mem::size_of::<TOKEN_ELEVATION>() as u32
            || elevation.TokenIsElevated != 0
        {
            return Err("启动 Flower 拒绝提升令牌；请由原始普通用户运行。".into());
        }
    }
    Ok(())
}

fn start_input_method() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        require_non_elevated()?;
        flower::platform::registration::enable().map_err(|e| e.to_string())?;
        println!("已为当前用户启用 Flower，请按 Win+Space 手动切换；尚未切换当前焦点的输入法。");
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    Err("启动 Flower 输入法仅支持 Windows 10/11 x64。".to_owned())
}

fn execute(command: Command) -> Result<(), String> {
    match command {
        Command::StartInputMethod => return start_input_method(),
        Command::Help => {
            println!("{USAGE}");
            return Ok(());
        }
        Command::Unicode(scalar) => {
            print!("{scalar}");
            return Ok(());
        }
        Command::Gui => {
            #[cfg(target_os = "windows")]
            return flower::platform::settings_ui::run_settings_app();
            #[cfg(not(target_os = "windows"))]
            return Err("Flower 设置窗口目前仅支持 Windows 10/11 x64。".to_owned());
        }
        _ => {}
    }
    let store = DictionaryStore::for_current_user().map_err(|e| e.to_string())?;
    let result = match command {
        Command::Status => {
            let data = store.load().map_err(|e| e.to_string())?;
            println!(
                "格式版本：{}\n代次：{}\n记住选词顺序：{}\n个人词条：{}\n排序作用域：{}",
                data.schema_version,
                data.generation,
                if data.adaptive_ranking_enabled {
                    "开启"
                } else {
                    "关闭"
                },
                data.entries.len(),
                data.rankings.len()
            );
            for item in data.domains {
                println!(
                    "专业词库：{}（{}，{} 条）",
                    item.name,
                    if item.enabled { "启用" } else { "停用" },
                    item.entries.len()
                );
            }
            return Ok(());
        }
        Command::Import(path, name) => store.import_tsv(&path, name.as_deref()),
        Command::Export(path, name) => store.export_tsv(&path, name.as_deref()),
        Command::Remove(pinyin, word, name) => store.remove_entry(&pinyin, &word, name.as_deref()),
        Command::Domain(name, enabled) => store.set_domain_enabled(&name, enabled),
        Command::Learning(enabled) => store.set_learning_enabled(enabled),
        Command::ClearLearning => store.clear_learning(),
        _ => unreachable!(),
    };
    result.map_err(|e| e.to_string())?;
    println!("操作完成。修改后请重新切换到 Flower 输入法使设置生效。");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let command = parse(&args);
    let only_start = matches!(&command, Ok(Command::StartInputMethod));
    match command.and_then(execute) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Flower 设置程序失败：{error}\n\n{USAGE}");
            if only_start {
                #[cfg(target_os = "windows")]
                unsafe {
                    use windows::core::w;
                    use windows::Win32::UI::WindowsAndMessaging::{
                        MessageBoxW, MB_ICONERROR, MB_OK,
                    };
                    MessageBoxW(
                        None,
                        w!("Flower 启动或启用失败，设置窗口可能未能打开。请以普通用户身份从开始菜单运行“启用 Flower（当前用户）”并打开 Flower 设置重试；若已启用，可按 Win+Space 手动选择 Flower。"),
                        w!("Flower 启动失败"),
                        MB_OK | MB_ICONERROR,
                    );
                }
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn command(args: &[&str]) -> Result<Command, String> {
        parse(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }
    #[test]
    fn strict_arguments() {
        assert_eq!(command(&[]).unwrap(), Command::Gui);
        assert_eq!(
            command(&["--start-input-method"]).unwrap(),
            Command::StartInputMethod
        );
        assert!(command(&["--start-input-method", "extra"]).is_err());
        assert!(command(&["--start-input-method", "--help"]).is_err());
        assert!(command(&["dictionary", "status", "--help"]).is_err());
        assert!(command(&["--help", "extra"]).is_err());
        assert!(command(&["dictionary", "learning", "yes"]).is_err());
        assert!(command(&["dictionary", "import", "a.tsv", "--domain"]).is_err());
        assert_eq!(
            command(&["dictionary", "remove", "ni'hao", "你好", "--domain", "医学"]).unwrap(),
            Command::Remove("ni'hao".into(), "你好".into(), Some("医学".into()))
        );
    }
    #[test]
    fn unicode_validation() {
        assert_eq!(unicode_scalar("U+20000").unwrap(), '\u{20000}');
        for value in [
            "U+D800", "U+110000", "U+0000", "U+007F", "U+009F", "U+", "U+-1", "20000",
        ] {
            assert!(unicode_scalar(value).is_err(), "{value}");
        }
    }
    #[cfg(windows)]
    #[test]
    fn non_unicode_paths_are_preserved() {
        use std::os::windows::ffi::OsStringExt;
        let path = OsString::from_wide(&[0xd800, 0x2e, 0x74]);
        assert_eq!(
            parse(&["dictionary".into(), "import".into(), path.clone()]).unwrap(),
            Command::Import(PathBuf::from(path.clone()), None)
        );
        assert!(parse(&["dictionary".into(), "domain".into(), path, "on".into()]).is_err());
    }
}
