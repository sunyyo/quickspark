use crate::{
    model::{Action, Query, ResultItem, SettingChange},
    storage::commands_file,
};

use super::{Provider, ProviderContext, ProviderOutput, file_search::everything_available};

const PROVIDER_ID: &str = "settings";

pub(crate) struct SettingsProvider;

impl SettingsProvider {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Provider for SettingsProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, context: &ProviderContext<'_>) -> ProviderOutput {
        let Some(terms) = settings_query(query.raw()) else {
            return ProviderOutput::empty();
        };
        let settings = context.settings;
        let mut entries = vec![
            setting_result(
                format!("唤起快捷键：{}（Enter 切换）", settings.hotkey.label()),
                "快捷键 热键 hotkey alt ctrl space 唤起",
                SettingChange::CycleHotkey,
            ),
            setting_result(
                format!(
                    "开机自动启动：{}",
                    if settings.auto_start {
                        "已开启"
                    } else {
                        "已关闭"
                    }
                ),
                "开机 自动启动 自启动 startup boot",
                SettingChange::ToggleAutoStart,
            ),
            setting_result(
                format!(
                    "界面主题：{}",
                    if context.light_theme {
                        "亮色"
                    } else {
                        "暗色"
                    }
                ),
                "界面 主题 亮色 暗色 theme",
                SettingChange::ToggleTheme,
            ),
            setting_result(
                format!("最多保留结果：{} 条", settings.result_limit),
                "结果 数量 条数 result limit",
                SettingChange::CycleResultLimit,
            ),
            setting_result(
                format!("默认网页搜索：{}", settings.web_engine.label()),
                "网页 搜索引擎 默认 百度 必应 google web",
                SettingChange::CycleWebEngine,
            ),
            SettingEntry {
                title: "搜索前缀：查看并试用所有功能前缀".to_owned(),
                keywords: "搜索 前缀 命令 prefix help",
                action: Some(Action::SetQuery {
                    query: "? 搜索".to_owned(),
                }),
            },
            SettingEntry {
                title: "AI 问答、Tavily 联网搜索与翻译：配置模型和 API Key".to_owned(),
                keywords: "ai 人工智能 大模型 翻译 tavily 联网 搜索 查证 api key 模型",
                action: Some(Action::SetQuery {
                    query: "ai settings".to_owned(),
                }),
            },
        ];
        if let Some(path) = commands_file() {
            entries.push(SettingEntry {
                title: "自定义指令：打开 commands.txt 编辑".to_owned(),
                keywords: "自定义 指令 命令 commands edit 配置",
                action: Some(Action::OpenPath { path }),
            });
        }
        entries.push(SettingEntry {
            title: format!(
                "Everything 文件索引：{}",
                if everything_available() {
                    "运行正常"
                } else {
                    "未运行"
                }
            ),
            keywords: "everything 文件 索引 状态",
            action: None,
        });

        let results = entries
            .into_iter()
            .filter(|entry| entry.matches(terms))
            .enumerate()
            .map(|(position, entry)| ResultItem {
                provider_id: PROVIDER_ID,
                title: entry.title,
                score: 100_000 - position as i32,
                actions: entry.action.into_iter().collect(),
            })
            .collect::<Vec<_>>();
        let results = if results.is_empty() {
            vec![ResultItem {
                provider_id: PROVIDER_ID,
                title: format!("没有找到“{terms}”相关设置"),
                score: 100_000,
                actions: vec![Action::SetQuery {
                    query: "settings".to_owned(),
                }],
            }]
        } else {
            results
        };
        ProviderOutput {
            results,
            exclusive: true,
        }
    }
}

struct SettingEntry {
    title: String,
    keywords: &'static str,
    action: Option<Action>,
}

impl SettingEntry {
    fn matches(&self, terms: &str) -> bool {
        if terms.is_empty() {
            return true;
        }
        let searchable = format!("{} {}", self.title, self.keywords).to_lowercase();
        terms
            .to_lowercase()
            .split_whitespace()
            .all(|term| searchable.contains(term))
    }
}

fn setting_result(title: String, keywords: &'static str, change: SettingChange) -> SettingEntry {
    SettingEntry {
        title,
        keywords,
        action: Some(Action::ChangeSetting { change }),
    }
}

fn settings_query(input: &str) -> Option<&str> {
    let input = input.trim();
    if ["settings", "setting", "set", "设置"]
        .iter()
        .any(|prefix| input.eq_ignore_ascii_case(prefix))
    {
        return Some("");
    }
    let split_at = input.find(char::is_whitespace)?;
    let (prefix, terms) = input.split_at(split_at);
    ["settings", "setting", "set", "设置"]
        .iter()
        .any(|candidate| prefix.eq_ignore_ascii_case(candidate))
        .then_some(terms.trim())
}

#[cfg(test)]
mod tests {
    use super::settings_query;

    #[test]
    fn recognizes_only_explicit_settings_commands() {
        assert_eq!(settings_query("settings"), Some(""));
        assert_eq!(settings_query("set 主题"), Some("主题"));
        assert_eq!(settings_query("设置 快捷键"), Some("快捷键"));
        assert_eq!(settings_query("setup"), None);
    }
}
