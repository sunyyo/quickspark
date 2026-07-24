use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Query {
    raw: String,
    normalized: String,
}

impl Query {
    pub(crate) fn new(input: &str) -> Self {
        let raw = input.trim().to_owned();
        let normalized = raw.to_lowercase();
        Self { raw, normalized }
    }

    pub(crate) fn raw(&self) -> &str {
        &self.raw
    }

    pub(crate) fn normalized(&self) -> &str {
        &self.normalized
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AiTask {
    Question,
    Translation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AiRequest {
    pub(crate) display_prompt: String,
    pub(crate) request_prompt: String,
    pub(crate) result_label: String,
    pub(crate) task: AiTask,
    pub(crate) cache_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    LaunchApplication {
        path: PathBuf,
        usage_key: String,
    },
    LaunchPackagedApplication {
        app_id: String,
        usage_key: String,
    },
    RunAsAdministrator {
        path: PathBuf,
    },
    OpenContainingFolder {
        path: PathBuf,
    },
    OpenPath {
        path: PathBuf,
    },
    OpenUrl {
        url: String,
    },
    OpenTarget {
        target: String,
    },
    CopyText {
        text: String,
    },
    ActivateWindow {
        handle: isize,
    },
    SetQuery {
        query: String,
    },
    SetPinned {
        key: String,
        pinned: bool,
    },
    ChangeSetting {
        change: SettingChange,
    },
    SystemCommand {
        command: SystemCommand,
    },
    AskAi {
        request: AiRequest,
    },
    RegenerateAi {
        request: AiRequest,
    },
    CancelAi,
    ShowAiAnswer {
        prompt: String,
        answer: String,
        reasoning: String,
    },
    ConfigureAiKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingChange {
    CycleHotkey,
    ToggleAutoStart,
    ToggleTheme,
    CycleResultLimit,
    CycleWebEngine,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SystemCommand {
    Lock,
    Sleep,
    SignOut,
    Shutdown,
    Restart,
}

impl Action {
    pub(crate) fn menu_text(&self) -> &'static str {
        match self {
            Self::LaunchApplication { .. } | Self::LaunchPackagedApplication { .. } => "打开",
            Self::RunAsAdministrator { .. } => "以管理员身份运行\tCtrl+Enter",
            Self::OpenContainingFolder { .. } => "打开所在位置\tCtrl+Shift+E",
            Self::OpenPath { .. } => "打开",
            Self::OpenUrl { .. } => "在浏览器中打开",
            Self::OpenTarget { .. } => "打开",
            Self::CopyText { .. } => "复制\tCtrl+C",
            Self::ActivateWindow { .. } => "切换到窗口",
            Self::SetQuery { .. } => "填入示例",
            Self::SetPinned { pinned: true, .. } => "固定到顶部",
            Self::SetPinned { pinned: false, .. } => "取消固定",
            Self::ChangeSetting { .. } => "更改设置",
            Self::SystemCommand { .. } => "执行系统命令",
            Self::AskAi { .. } => "发送给 AI",
            Self::RegenerateAi { .. } => "重新生成",
            Self::CancelAi => "取消本次请求",
            Self::ShowAiAnswer { .. } => "查看完整回答",
            Self::ConfigureAiKey => "安全设置 API Key",
        }
    }

    pub(crate) fn favorite_key(&self) -> Option<String> {
        match self {
            Self::LaunchApplication { usage_key, .. }
            | Self::LaunchPackagedApplication { usage_key, .. } => Some(format!("app:{usage_key}")),
            Self::OpenPath { path } => {
                Some(format!("path:{}", path.to_string_lossy().to_lowercase()))
            }
            Self::OpenUrl { url } => Some(format!("url:{}", url.to_lowercase())),
            Self::OpenTarget { target } => Some(format!("target:{}", target.to_lowercase())),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResultItem {
    pub(crate) provider_id: &'static str,
    pub(crate) title: String,
    pub(crate) score: i32,
    pub(crate) actions: Vec<Action>,
}

impl ResultItem {
    pub(crate) fn primary_action(&self) -> Option<&Action> {
        self.actions.first()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    pub(crate) count: u32,
    pub(crate) last: u64,
}
