use std::process::ExitCode;

const USAGE: &str = "\
Flower 设置程序

用法：
  flower-settings          打开设置窗口
  flower-settings --help   显示此帮助

可设置候选页大小（1-99）和候选窗主题。保存后，请重新切换到 Flower
输入法使设置生效。";

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if !arguments.is_empty() {
        eprintln!("不支持的参数。\n\n{USAGE}");
        return ExitCode::FAILURE;
    }

    #[cfg(target_os = "windows")]
    {
        match flower::platform::settings_ui::run_settings_app() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Flower 设置程序失败：{error}");
                ExitCode::FAILURE
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        eprintln!("Flower 设置程序目前仅支持 Windows 10/11 x64。");
        ExitCode::FAILURE
    }
}
