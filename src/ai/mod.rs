//! Small, runtime-independent contracts for optional offline correction.
//! No inference runtime, host document access or network belongs here.

use serde::{Deserialize, Serialize};

pub mod ipc;

pub const AI_COMMENT: &str = "[Flower AI]";
pub const MIN_TEXT_CHARS: usize = 4;
pub const MAX_TEXT_CHARS: usize = 64;
pub const MAX_FRAME_BYTES: usize = 4096;
pub const DEBOUNCE_MS: u64 = 250;
pub const RESULT_TIMEOUT_MS: u64 = 5000;
pub const SERVER_IDLE_SECONDS: u64 = 60;

/// A fixed refusal reason; contains no document text, paths, or host metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeBlockReason {
    Selection,
    Property,
    Value,
    Undeclared,
    InvalidValue,
    Interface,
    Read,
    Empty,
    InvalidList,
    Sensitive,
    Unsupported,
}

impl ScopeBlockReason {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Selection => "AI 范围受限 [S01]：无法取得当前选区",
            Self::Property => "AI 范围受限 [S02]：应用范围属性不可用",
            Self::Value => "AI 范围受限 [S03]：无法读取范围声明",
            Self::Undeclared => "AI 范围受限 [S04]：应用未提供范围声明",
            Self::InvalidValue => "AI 范围受限 [S05]：范围声明格式无效",
            Self::Interface => "AI 范围受限 [S06]：应用范围接口不可用",
            Self::Read => "AI 范围受限 [S07]：应用范围查询失败",
            Self::Empty => "AI 范围受限 [S08]：应用返回空范围列表",
            Self::InvalidList => "AI 范围受限 [S09]：应用范围列表无效",
            Self::Sensitive => "AI 范围受限 [S10]：密码、PIN或私密输入框",
            Self::Unsupported => "AI 范围受限 [S11]：当前输入类型未支持",
        }
    }
}

/// Fixed, text-free live diagnostics. Never a candidate or permission to infer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiStatus {
    Disabled,
    Unavailable,
    ModulePathFailed,
    UnsupportedPath,
    UnsafeDirectory,
    MissingHelper,
    MissingAssets,
    ClientFailed,
    TimerFailed,
    Waiting,
    Ineligible,
    ScopeBlocked(ScopeBlockReason),
    Checking,
    NoSuggestion,
    Suggested,
    Failed,
    EditFailed,
}

impl AiStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Disabled => "AI 已关闭：在 Flower 设置中开启",
            Self::Unavailable => "AI 组件不可用：检查安装目录与完整包",
            Self::ModulePathFailed => "AI 不可用：无法定位输入法模块",
            Self::UnsupportedPath => "AI 不可用：安装路径形式不支持",
            Self::UnsafeDirectory => "AI 不可用：安装目录安全检查失败",
            Self::MissingHelper => "AI 不可用：安装目录缺少辅助程序",
            Self::MissingAssets => "AI 不可用：安装目录缺少模型目录",
            Self::ClientFailed => "AI 不可用：后台客户端初始化失败",
            Self::TimerFailed => "AI 不可用：空闲定时器初始化失败",
            Self::Waiting => "AI 等待停顿（需首页首选）",
            Self::Ineligible => "AI 等待 4–64 字完整中文候选",
            Self::ScopeBlocked(reason) => reason.label(),
            Self::Checking => "AI 检查中：首次加载最多等待 5 秒",
            Self::NoSuggestion => "AI 已检查：无纠正建议",
            Self::Suggested => "AI 已提供 ✓ 纠正建议，请手动选择",
            Self::Failed => "AI 超时或组件故障：可重新输入后重试",
            Self::EditFailed => "AI 无法取得只读编辑会话",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiInput {
    pub raw: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub id: u64,
    pub corrected: Option<String>,
}

pub fn eligible_text(text: &str) -> bool {
    let count = text.chars().count();
    (MIN_TEXT_CHARS..=MAX_TEXT_CHARS).contains(&count)
        && text.chars().all(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// Structural safety check, separate from model confidence filtering.
pub fn valid_correction(source: &str, corrected: &str) -> bool {
    if !eligible_text(source)
        || !eligible_text(corrected)
        || source.chars().count() != corrected.chars().count()
    {
        return false;
    }
    let changed = source
        .chars()
        .zip(corrected.chars())
        .filter(|(a, b)| a != b)
        .count();
    changed > 0 && changed <= 2 && changed * 4 <= source.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn correction_bounds_reject_rewrites_and_non_han() {
        assert!(valid_correction("今天新情很好", "今天心情很好"));
        assert!(!valid_correction("今天心情很好", "今天心情很好"));
        assert!(!valid_correction("今天心情很好", "今天心情很好呀"));
        assert!(!valid_correction("今天新情很好", "今天心情很A"));
        assert!(!eligible_text("nihao"));
        assert!(!eligible_text("𠮷野家很好"));
        assert!(!valid_correction("今天新情很好", "天气实在不好"));
    }
}
