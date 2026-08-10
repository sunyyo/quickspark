use std::{
    collections::{HashMap, HashSet},
    env, fs,
    net::IpAddr,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
        RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
    },
};

use crate::model::Usage;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "QuickSpark";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HotkeyChoice {
    AltSpace,
    ControlSpace,
    AltQ,
    ControlAltSpace,
}

impl HotkeyChoice {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::AltSpace => "Alt+Space",
            Self::ControlSpace => "Ctrl+Space",
            Self::AltQ => "Alt+Q",
            Self::ControlAltSpace => "Ctrl+Alt+Space",
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::AltSpace => Self::ControlSpace,
            Self::ControlSpace => Self::AltQ,
            Self::AltQ => Self::ControlAltSpace,
            Self::ControlAltSpace => Self::AltSpace,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::AltSpace => "alt_space",
            Self::ControlSpace => "control_space",
            Self::AltQ => "alt_q",
            Self::ControlAltSpace => "control_alt_space",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "alt_space" => Some(Self::AltSpace),
            "control_space" => Some(Self::ControlSpace),
            "alt_q" => Some(Self::AltQ),
            "control_alt_space" => Some(Self::ControlAltSpace),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WebEngine {
    Baidu,
    Bing,
    Google,
}

impl WebEngine {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Baidu => "百度",
            Self::Bing => "必应",
            Self::Google => "Google",
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::Baidu => Self::Bing,
            Self::Bing => Self::Google,
            Self::Google => Self::Baidu,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Baidu => "baidu",
            Self::Bing => "bing",
            Self::Google => "google",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "baidu" => Some(Self::Baidu),
            "bing" => Some(Self::Bing),
            "google" => Some(Self::Google),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Settings {
    pub(crate) hotkey: HotkeyChoice,
    pub(crate) auto_start: bool,
    pub(crate) result_limit: usize,
    pub(crate) web_engine: WebEngine,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AiConfig {
    pub(crate) api_url: String,
    pub(crate) model: String,
    pub(crate) system_prompt: String,
    pub(crate) max_tokens: u32,
    pub(crate) timeout_seconds: u32,
    pub(crate) thinking_mode: ThinkingMode,
    pub(crate) reasoning_effort: ReasoningEffort,
}

const DEFAULT_SYSTEM_PROMPT: &str = "你是一名严谨、实用的中文 AI 助手。直接回答用户的核心问题，默认使用简体中文。简单问题简洁作答；复杂问题使用清晰的结构、步骤和必要示例。不得编造事实、来源或执行结果；不确定时明确说明，并给出验证方法。提供建议时应具体、可执行，并指出重要前提与风险。避免重复问题、空泛开场和不必要的总结。严格遵循用户指定的格式、语言和篇幅。";
const LEGACY_SYSTEM_PROMPT: &str = "你是一个简洁、可靠的中文助手。";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ThinkingMode {
    #[default]
    Auto,
    Enabled,
    Disabled,
}

impl ThinkingMode {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "enabled" | "on" | "true" => Some(Self::Enabled),
            "disabled" | "off" | "false" => Some(Self::Disabled),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ReasoningEffort {
    #[default]
    High,
    Max,
}

impl ReasoningEffort {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Max => "max",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "high" => Some(Self::High),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            api_url: String::new(),
            model: String::new(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_owned(),
            max_tokens: 8192,
            timeout_seconds: 60,
            thinking_mode: ThinkingMode::Auto,
            reasoning_effort: ReasoningEffort::High,
        }
    }
}

impl AiConfig {
    pub(crate) fn is_ready(&self) -> bool {
        let url = self.api_url.trim();
        !self.model.trim().is_empty() && is_allowed_ai_url(url)
    }
}

fn is_allowed_ai_url(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(authority) = url
        .strip_prefix("http://")
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .filter(|authority| !authority.is_empty() && !authority.contains('@'))
    else {
        return false;
    };
    let host = if let Some(ipv6) = authority.strip_prefix('[') {
        ipv6.split_once(']').map(|(host, _)| host).unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host.trim_end_matches('.').eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: HotkeyChoice::AltSpace,
            auto_start: false,
            result_limit: 20,
            web_engine: WebEngine::Baidu,
        }
    }
}

fn data_file(name: &str) -> Option<PathBuf> {
    env::var("APPDATA")
        .ok()
        .map(|value| PathBuf::from(value).join("QuickSpark").join(name))
}

const LEGACY_DATA_FILES: [&str; 8] = [
    "theme.txt",
    "usage.tsv",
    "blocked.txt",
    "allowed.txt",
    "settings.txt",
    "commands.txt",
    "ai-settings.txt",
    "pinned.txt",
];
const LEGACY_MIGRATION_MARKER: &str = ".legacy-migration-complete";

pub(crate) fn migrate_legacy_data() {
    let Ok(appdata) = env::var("APPDATA") else {
        return;
    };
    let old = PathBuf::from(&appdata).join("QuickLauncher");
    let new = PathBuf::from(appdata).join("QuickSpark");
    let _ = migrate_legacy_data_dirs(&old, &new);
}

fn migrate_legacy_data_dirs(old: &Path, new: &Path) -> bool {
    if !old.exists() || new.join(LEGACY_MIGRATION_MARKER).is_file() {
        return true;
    }
    if !new.exists() && fs::rename(old, new).is_ok() {
        return true;
    }
    if fs::create_dir_all(new).is_err() {
        return false;
    }
    for name in LEGACY_DATA_FILES {
        let source = old.join(name);
        if !source.is_file() {
            continue;
        }
        let destination = new.join(name);
        if destination.is_file() {
            continue;
        }
        if destination.exists() || fs::copy(source, destination).is_err() {
            return false;
        }
    }
    write_atomic(
        &new.join(LEGACY_MIGRATION_MARKER),
        "legacy QuickLauncher data migration completed\n",
    )
}

fn load_patterns(file_name: &str, initial_comment: &str) -> Vec<String> {
    let Some(path) = data_file(file_name) else {
        return Vec::new();
    };
    if !path.exists() {
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let _ = fs::write(&path, initial_comment);
    }
    fs::read_to_string(path)
        .ok()
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(str::to_lowercase)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn load_blocked() -> Vec<String> {
    load_patterns("blocked.txt", "# 每行填写一个需要隐藏的程序名称或关键词\n")
}

pub(crate) fn load_allowed() -> Vec<String> {
    load_patterns(
        "allowed.txt",
        "# 每行填写一个需要强制显示的程序名称或关键词\n",
    )
}

pub(crate) fn load_usage() -> HashMap<String, Usage> {
    data_file("usage.tsv")
        .map(|path| load_usage_from(&path))
        .unwrap_or_default()
}

pub(crate) fn load_usage_from(path: &Path) -> HashMap<String, Usage> {
    let mut usage = HashMap::new();
    let Ok(text) = fs::read_to_string(path) else {
        return usage;
    };
    for line in text.lines() {
        let mut fields = line.split('\t');
        let (Some(key), Some(count), Some(last)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if let (Ok(count), Ok(last)) = (count.parse(), last.parse()) {
            usage.insert(key.to_owned(), Usage { count, last });
        }
    }
    usage
}

pub(crate) fn save_usage(usage: &HashMap<String, Usage>) {
    let Some(path) = data_file("usage.tsv") else {
        return;
    };
    let _ = save_usage_to(&path, usage);
}

pub(crate) fn save_usage_to(path: &Path, usage: &HashMap<String, Usage>) -> bool {
    let mut rows: Vec<_> = usage.iter().collect();
    rows.sort_by_key(|(key, _)| *key);
    let text = rows
        .into_iter()
        .map(|(key, value)| format!("{key}\t{}\t{}\n", value.count, value.last))
        .collect::<String>();
    write_atomic(path, &text)
}

fn write_atomic(path: &Path, contents: &str) -> bool {
    if let Some(dir) = path.parent()
        && fs::create_dir_all(dir).is_err()
    {
        return false;
    }
    let temporary = path.with_file_name(format!(
        "{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("quickspark")
    ));
    if fs::write(&temporary, contents).is_err() {
        return false;
    }

    let source = wide(&temporary.to_string_lossy());
    let destination = wide(&path.to_string_lossy());
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let _ = fs::remove_file(temporary);
        return false;
    }
    true
}

pub(crate) fn load_light_theme() -> bool {
    data_file("theme.txt")
        .and_then(|path| fs::read_to_string(path).ok())
        .is_some_and(|value| value.trim() == "light")
}

pub(crate) fn save_theme(light: bool) -> bool {
    let Some(path) = data_file("theme.txt") else {
        return false;
    };
    write_atomic(&path, if light { "light" } else { "dark" })
}

pub(crate) fn load_settings() -> Settings {
    let mut settings = Settings::default();
    if let Some(path) = data_file("settings.txt")
        && let Ok(text) = fs::read_to_string(path)
    {
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "hotkey" => {
                    if let Some(hotkey) = HotkeyChoice::parse(value.trim()) {
                        settings.hotkey = hotkey;
                    }
                }
                "result_limit" => {
                    if let Ok(limit @ (8 | 12 | 20)) = value.trim().parse() {
                        settings.result_limit = limit;
                    }
                }
                "web_engine" => {
                    if let Some(engine) = WebEngine::parse(value.trim()) {
                        settings.web_engine = engine;
                    }
                }
                _ => {}
            }
        }
    }
    settings.auto_start = is_autostart_enabled();
    settings
}

pub(crate) fn save_settings(settings: &Settings) -> bool {
    let Some(path) = data_file("settings.txt") else {
        return false;
    };
    let text = format!(
        "hotkey={}\nresult_limit={}\nweb_engine={}\n",
        settings.hotkey.key(),
        settings.result_limit,
        settings.web_engine.key()
    );
    write_atomic(&path, &text)
}

pub(crate) fn set_autostart(enabled: bool) -> bool {
    let run_key = wide(RUN_KEY);
    let value_name = wide(RUN_VALUE);
    let mut key: HKEY = std::ptr::null_mut();
    if unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            run_key.as_ptr(),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    } != ERROR_SUCCESS
    {
        return false;
    }
    let key = RegistryKey(key);
    if enabled {
        let Ok(executable) = env::current_exe() else {
            return false;
        };
        let value = wide(&format!("\"{}\"", executable.to_string_lossy()));
        unsafe {
            RegSetValueExW(
                key.0,
                value_name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr() as *const u8,
                (value.len() * std::mem::size_of::<u16>()) as u32,
            ) == ERROR_SUCCESS
        }
    } else {
        matches!(
            unsafe { RegDeleteValueW(key.0, value_name.as_ptr()) },
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND
        )
    }
}

fn is_autostart_enabled() -> bool {
    let Ok(executable) = env::current_exe() else {
        return false;
    };
    let run_key = wide(RUN_KEY);
    let value_name = wide(RUN_VALUE);
    let mut size = 0u32;
    if unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            run_key.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    } != ERROR_SUCCESS
        || size < 2
    {
        return false;
    }
    let mut value = vec![0u16; size.div_ceil(2) as usize];
    if unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            run_key.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            value.as_mut_ptr() as _,
            &mut size,
        )
    } != ERROR_SUCCESS
    {
        return false;
    }
    let stored = String::from_utf16_lossy(
        &value[..value
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(value.len())],
    );
    stored
        .trim()
        .trim_matches('"')
        .eq_ignore_ascii_case(&executable.to_string_lossy())
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { RegCloseKey(self.0) };
        }
    }
}

pub(crate) fn commands_file() -> Option<PathBuf> {
    let path = data_file("commands.txt")?;
    if !path.exists() {
        let template = "# QuickSpark 自定义指令：别名 | 标题 | 目标\n\
# 使用 ; 查看指令，例如：; gh rust\n\
# docs | 打开文档目录 | %USERPROFILE%\\Documents\n\
# gh | 搜索 GitHub | https://github.com/search?q={query}\n";
        if !write_atomic(&path, template) {
            return None;
        }
    }
    Some(path)
}

pub(crate) fn ai_config_file() -> Option<PathBuf> {
    let path = data_file("ai-settings.txt")?;
    if !path.exists() {
        let template = "# QuickSpark AI：OpenAI 兼容 Chat Completions 接口\n\
# API Key 请在 QuickSpark 中输入 ai key 安全设置，不要写在本文件。\n\
api_url=\n\
model=\n\
system_prompt=你是一名严谨、实用的中文 AI 助手。直接回答用户的核心问题，默认使用简体中文。简单问题简洁作答；复杂问题使用清晰的结构、步骤和必要示例。不得编造事实、来源或执行结果；不确定时明确说明，并给出验证方法。提供建议时应具体、可执行，并指出重要前提与风险。避免重复问题、空泛开场和不必要的总结。严格遵循用户指定的格式、语言和篇幅。\n\
max_tokens=8192\n\
timeout_seconds=60\n\
# DeepSeek V4 思考模式：auto / enabled / disabled；强度：high / max\n\
thinking_mode=auto\n\
reasoning_effort=high\n";
        if !write_atomic(&path, template) {
            return None;
        }
    }
    ensure_ai_thinking_settings(&path);
    Some(path)
}

fn ensure_ai_thinking_settings(path: &Path) {
    let Ok(mut text) = fs::read_to_string(path) else {
        return;
    };
    let legacy_prompt = format!("system_prompt={LEGACY_SYSTEM_PROMPT}");
    let current_prompt = format!("system_prompt={DEFAULT_SYSTEM_PROMPT}");
    let upgraded_prompt = text.contains(&legacy_prompt);
    if upgraded_prompt {
        text = text.replace(&legacy_prompt, &current_prompt);
    }
    let has_key = |expected: &str| {
        text.lines().any(|line| {
            let line = line.trim();
            !line.starts_with('#')
                && line
                    .split_once('=')
                    .is_some_and(|(key, _)| key.trim() == expected)
        })
    };
    let has_thinking_mode = has_key("thinking_mode");
    let has_reasoning_effort = has_key("reasoning_effort");
    if has_thinking_mode && has_reasoning_effort && !upgraded_prompt {
        return;
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("# DeepSeek V4 思考模式：auto / enabled / disabled；强度：high / max\n");
    if !has_thinking_mode {
        text.push_str("thinking_mode=auto\n");
    }
    if !has_reasoning_effort {
        text.push_str("reasoning_effort=high\n");
    }
    let _ = write_atomic(path, &text);
}

pub(crate) fn load_ai_config() -> AiConfig {
    let mut config = AiConfig::default();
    let Some(path) = ai_config_file() else {
        return config;
    };
    let Ok(text) = fs::read_to_string(path) else {
        return config;
    };
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "api_url" => config.api_url = value.to_owned(),
            "model" => config.model = value.to_owned(),
            "system_prompt" => config.system_prompt = value.to_owned(),
            "max_tokens" => {
                if let Ok(value @ 1..=65_536) = value.parse() {
                    config.max_tokens = value;
                }
            }
            "timeout_seconds" => {
                if let Ok(value @ 5..=300) = value.parse() {
                    config.timeout_seconds = value;
                }
            }
            "thinking_mode" => {
                if let Some(value) = ThinkingMode::parse(value) {
                    config.thinking_mode = value;
                }
            }
            "reasoning_effort" => {
                if let Some(value) = ReasoningEffort::parse(value) {
                    config.reasoning_effort = value;
                }
            }
            _ => {}
        }
    }
    config
}

pub(crate) fn load_pinned() -> HashSet<String> {
    data_file("pinned.txt")
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn save_pinned(pinned: &HashSet<String>) -> bool {
    let Some(path) = data_file("pinned.txt") else {
        return false;
    };
    let mut keys = pinned.iter().collect::<Vec<_>>();
    keys.sort();
    let text = keys
        .into_iter()
        .map(|key| format!("{key}\n"))
        .collect::<String>();
    write_atomic(&path, &text)
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{AiConfig, LEGACY_MIGRATION_MARKER, migrate_legacy_data_dirs};

    #[test]
    fn requires_http_url_and_model() {
        let mut config = AiConfig::default();
        assert!(!config.is_ready());
        config.api_url = "https://example.com/v1/chat/completions".to_owned();
        assert!(!config.is_ready());
        config.model = "example-model".to_owned();
        assert!(config.is_ready());
        config.api_url = "file:///unsafe".to_owned();
        assert!(!config.is_ready());
        config.api_url = "http://example.com/v1/chat/completions".to_owned();
        assert!(!config.is_ready());
        config.api_url = "http://127.0.0.1:11434/v1/chat/completions".to_owned();
        assert!(config.is_ready());
        config.api_url = "http://[::1]:11434/v1/chat/completions".to_owned();
        assert!(config.is_ready());
    }

    #[test]
    fn legacy_migration_preserves_new_files_and_retries_partial_failures() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "quickspark-migration-test-{}-{unique}",
            std::process::id()
        ));
        let old = root.join("QuickLauncher");
        let new = root.join("QuickSpark");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("settings.txt"), "old settings").unwrap();
        fs::write(old.join("theme.txt"), "dark").unwrap();
        fs::write(new.join("settings.txt"), "new settings").unwrap();

        let blocked_destination = new.join("theme.txt");
        fs::create_dir(&blocked_destination).unwrap();
        assert!(!migrate_legacy_data_dirs(&old, &new));
        assert!(!new.join(LEGACY_MIGRATION_MARKER).exists());

        fs::remove_dir(&blocked_destination).unwrap();
        assert!(migrate_legacy_data_dirs(&old, &new));
        assert_eq!(
            fs::read_to_string(new.join("settings.txt")).unwrap(),
            "new settings"
        );
        assert_eq!(fs::read_to_string(new.join("theme.txt")).unwrap(), "dark");
        assert!(new.join(LEGACY_MIGRATION_MARKER).is_file());

        fs::remove_dir_all(root).unwrap();
    }
}
