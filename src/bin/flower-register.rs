use std::ffi::OsString;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
use std::path::PathBuf;
use std::process::ExitCode;

use flower::registration::{parse_arguments, CliCommand};

const USAGE: &str = "\
Flower 注册工具（Windows x64）

用法：
  flower-register check-install-path --path <目录>  只读检查机器安装目录安全性
  flower-register prepare-install-path --path <目录>  安全创建缺失安装目录（管理员）
  flower-register register [--dll <path>]  注册机器级 COM/TSF 组件（管理员终端）
  flower-register unregister               注销机器级 COM/TSF 组件（管理员终端）
  flower-register enable                   为当前用户添加 Flower 输入法
  flower-register disable                  从当前用户移除 Flower 输入法
  flower-register --help                   显示此帮助

说明：
  register 默认使用本程序所在目录的 flower.dll，不依赖当前工作目录。
  register 和 unregister 必须从“以管理员身份运行”的终端执行，不会自动提权。
  机器级注册不会自动为所有用户启用。需要使用的用户应各自运行 enable，
  然后按 Win+Space 手动选择 Flower。
  注销前，各用户应先运行 disable。unregister 不会删除 DLL、用户设置或
  其他用户的输入配置；已加载 Flower 的进程可能需要退出或重启后才能替换 DLL。";

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<OsString>>();
    let command = match parse_arguments(arguments) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("Flower 注册工具：{error}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    if command == CliCommand::Help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    match run_command(command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Flower 注册工具失败：{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn run_command(command: CliCommand) -> Result<(), String> {
    use flower::platform::registration;
    use flower::registration::OperationOutcome;

    match command {
        CliCommand::CheckInstallPath { path } => {
            flower::platform::install_path::check_install_path(&PathBuf::from(path))?;
            println!("安装目录安全检查通过；未创建目录或修改权限。");
        }
        CliCommand::PrepareInstallPath { path } => {
            flower::platform::install_path::prepare_install_path(&PathBuf::from(path))?;
            println!("安装目录已安全准备并复查；未更改已有目录权限或内容。");
        }
        CliCommand::Register { dll } => {
            let dll = resolve_dll_path(dll)?;
            let outcome = registration::register(&dll).map_err(|error| error.to_string())?;
            match outcome {
                OperationOutcome::Changed => {
                    println!(
                        "Flower 机器级 x64 COM/TSF 注册已完成。\n\
                         注册没有自动为所有用户启用输入法。请在需要使用的普通用户终端运行\n\
                         flower-register enable，然后按 Win+Space 手动选择 Flower。"
                    );
                }
                OperationOutcome::Unchanged => {
                    println!(
                        "Flower 已使用相同 DLL 完成机器级注册，未重复写入。\n\
                         当前用户仍需单独运行 flower-register enable。"
                    );
                }
            }
        }
        CliCommand::Unregister => {
            let outcome = registration::unregister().map_err(|error| error.to_string())?;
            match outcome {
                OperationOutcome::Changed => {
                    println!(
                        "Flower 机器级 COM/TSF 注册已注销。\n\
                         未删除 flower.dll、用户设置或其他用户的配置。"
                    );
                }
                OperationOutcome::Unchanged => {
                    println!("Flower 机器级注册已不存在，无需重复注销。");
                }
            }
        }
        CliCommand::Enable => {
            registration::enable().map_err(|error| error.to_string())?;
            println!(
                "已为当前 Windows 用户添加 Flower 输入配置。\n\
                 这不表示它已在当前焦点应用中被激活；请按 Win+Space 手动选择 Flower，必要时注销并重新登录。"
            );
        }
        CliCommand::Disable => {
            registration::disable().map_err(|error| error.to_string())?;
            println!(
                "已从当前 Windows 用户移除 Flower 输入配置。\n\
                 不会删除机器级注册、DLL 或设置。"
            );
        }
        CliCommand::Help => unreachable!("帮助在平台分派前处理"),
    }
    Ok(())
}

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
fn run_command(_command: CliCommand) -> Result<(), String> {
    Err("Flower 注册工具仅支持 Windows 10/11 x64；--help 可在其他平台使用。".to_owned())
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn resolve_dll_path(explicit: Option<OsString>) -> Result<PathBuf, String> {
    use flower::registration::default_dll_path;

    if let Some(path) = explicit {
        return Ok(PathBuf::from(path));
    }
    let executable =
        std::env::current_exe().map_err(|error| format!("无法确定注册工具自身路径：{error}"))?;
    default_dll_path(&executable).map_err(|error| error.to_string())
}
