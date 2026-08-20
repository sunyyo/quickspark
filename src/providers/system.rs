use crate::model::{Action, Query, ResultItem, SystemCommand};

use super::{Provider, ProviderContext, ProviderOutput};

const PROVIDER_ID: &str = "system";

pub(crate) struct SystemProvider;

impl Provider for SystemProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, _context: &ProviderContext<'_>) -> ProviderOutput {
        let Some((mode, terms)) = system_query(query.raw()) else {
            return ProviderOutput::empty();
        };
        let entries = match mode {
            QueryMode::Commands => system_commands(),
            QueryMode::Settings => windows_settings(),
        };
        let results = entries
            .into_iter()
            .filter(|entry| entry.matches(terms))
            .enumerate()
            .map(|(position, entry)| ResultItem {
                provider_id: PROVIDER_ID,
                title: entry.title.to_owned(),
                score: 100_000 - position as i32,
                actions: vec![entry.action],
            })
            .collect::<Vec<_>>();
        let results = if results.is_empty() {
            vec![ResultItem {
                provider_id: PROVIDER_ID,
                title: format!("没有找到“{terms}”相关系统项目"),
                score: 100_000,
                actions: Vec::new(),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueryMode {
    Commands,
    Settings,
}

struct SystemEntry {
    title: &'static str,
    keywords: &'static str,
    action: Action,
}

impl SystemEntry {
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

fn system_commands() -> Vec<SystemEntry> {
    vec![
        command("锁定电脑", "锁屏 lock workstation", SystemCommand::Lock),
        command(
            "睡眠（执行前确认）",
            "睡眠 休眠 sleep",
            SystemCommand::Sleep,
        ),
        command(
            "注销当前用户（执行前确认）",
            "注销 退出登录 sign out logoff",
            SystemCommand::SignOut,
        ),
        command(
            "关闭电脑（执行前确认）",
            "关机 关闭 shutdown power off",
            SystemCommand::Shutdown,
        ),
        command(
            "重新启动电脑（执行前确认）",
            "重启 重新启动 restart reboot",
            SystemCommand::Restart,
        ),
        SystemEntry {
            title: "打开回收站",
            keywords: "回收站 recycle bin trash",
            action: Action::OpenTarget {
                target: "shell:RecycleBinFolder".to_owned(),
            },
        },
        SystemEntry {
            title: "打开 Windows 设置",
            keywords: "系统 设置 settings windows",
            action: Action::OpenTarget {
                target: "ms-settings:".to_owned(),
            },
        },
    ]
}

fn command(title: &'static str, keywords: &'static str, command: SystemCommand) -> SystemEntry {
    SystemEntry {
        title,
        keywords,
        action: Action::SystemCommand { command },
    }
}

fn windows_settings() -> Vec<SystemEntry> {
    [
        (
            "显示设置",
            "显示 屏幕 分辨率 缩放 display",
            "ms-settings:display",
        ),
        ("蓝牙和设备", "蓝牙 设备 bluetooth", "ms-settings:bluetooth"),
        (
            "网络和 Internet",
            "网络 internet wifi 以太网",
            "ms-settings:network-status",
        ),
        (
            "声音设置",
            "声音 音量 麦克风 sound audio",
            "ms-settings:sound",
        ),
        (
            "已安装的应用",
            "应用 卸载 软件 apps",
            "ms-settings:appsfeatures",
        ),
        (
            "启动应用",
            "开机启动 自启动 startup",
            "ms-settings:startupapps",
        ),
        (
            "Windows 更新",
            "系统 更新 update",
            "ms-settings:windowsupdate",
        ),
        ("隐私和安全性", "隐私 安全 privacy", "ms-settings:privacy"),
        (
            "存储设置",
            "磁盘 存储 空间 storage",
            "ms-settings:storagesense",
        ),
        (
            "个性化设置",
            "壁纸 颜色 主题 personalization",
            "ms-settings:personalization",
        ),
    ]
    .into_iter()
    .map(|(title, keywords, target)| SystemEntry {
        title,
        keywords,
        action: Action::OpenTarget {
            target: target.to_owned(),
        },
    })
    .collect()
}

fn system_query(input: &str) -> Option<(QueryMode, &str)> {
    let input = input.trim();
    if let Some(terms) = input.strip_prefix('$') {
        return Some((QueryMode::Settings, terms.trim()));
    }
    if ["sys", "system", "系统"]
        .iter()
        .any(|prefix| input.eq_ignore_ascii_case(prefix))
    {
        return Some((QueryMode::Commands, ""));
    }
    let split_at = input.find(char::is_whitespace)?;
    let (prefix, terms) = input.split_at(split_at);
    ["sys", "system", "系统"]
        .iter()
        .any(|candidate| prefix.eq_ignore_ascii_case(candidate))
        .then_some((QueryMode::Commands, terms.trim()))
}

#[cfg(test)]
mod tests {
    use super::{QueryMode, system_query};

    #[test]
    fn recognizes_system_and_settings_prefixes() {
        assert_eq!(system_query("sys"), Some((QueryMode::Commands, "")));
        assert_eq!(
            system_query("sys lock"),
            Some((QueryMode::Commands, "lock"))
        );
        assert_eq!(system_query("$ 蓝牙"), Some((QueryMode::Settings, "蓝牙")));
        assert_eq!(system_query("systematic"), None);
    }
}
