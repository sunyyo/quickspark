use std::{
    env, fs,
    io::Read,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::{
    model::{Action, Query, ResultItem},
    search::{rank_entry, search_keys},
    storage::{load_allowed, load_blocked},
};

use super::{Provider, ProviderContext, ProviderOutput};

const PROVIDER_ID: &str = "applications";

#[derive(Clone)]
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) target: EntryTarget,
    pub(crate) key: String,
    pub(crate) pinyin: String,
    pub(crate) initials: String,
}

#[derive(Clone)]
pub(crate) enum EntryTarget {
    File(PathBuf),
    Packaged(String),
}

pub(crate) struct ApplicationProvider {
    entries: Vec<Entry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IndexError {
    NoReadableRoots,
    Panic,
}

impl ApplicationProvider {
    pub(crate) fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_entries(entries: Vec<Entry>) -> Self {
        Self { entries }
    }

    pub(crate) fn build() -> Result<Self, IndexError> {
        let mut entries = Vec::new();
        let blocked = load_blocked();
        let allowed = load_allowed();
        let mut roots = Vec::new();
        if let Ok(value) = env::var("APPDATA") {
            roots.push(PathBuf::from(value).join(r"Microsoft\Windows\Start Menu\Programs"));
        }
        if let Ok(value) = env::var("PROGRAMDATA") {
            roots.push(PathBuf::from(value).join(r"Microsoft\Windows\Start Menu\Programs"));
        }
        if let Ok(value) = env::var("USERPROFILE") {
            roots.push(PathBuf::from(value).join("Desktop"));
        }
        if let Ok(value) = env::var("PUBLIC") {
            roots.push(PathBuf::from(value).join("Desktop"));
        }

        let mut readable_roots = 0;
        for root in roots {
            if collect_dir(&root, &mut entries, &blocked, &allowed, 8) {
                readable_roots += 1;
            }
        }
        let packaged_apps_readable = collect_packaged_apps(&mut entries, &blocked, &allowed);
        if readable_roots == 0 && !packaged_apps_readable {
            return Err(IndexError::NoReadableRoots);
        }

        entries.sort_by_key(|entry| entry.name.to_lowercase());
        // Windows commonly exposes the same shortcut through user/common Start Menu
        // and Desktop locations. For a launcher, identical display names are one app.
        entries.dedup_by(|left, right| left.key == right.key);
        Ok(Self { entries })
    }
}

impl Provider for ApplicationProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, context: &ProviderContext<'_>) -> ProviderOutput {
        let results = self
            .entries
            .iter()
            .filter_map(|entry| {
                let usage = context.usage.get(&entry.key).copied().unwrap_or_default();
                let score = entry_score(entry, query.normalized(), usage, context.now)?;
                let actions = match &entry.target {
                    EntryTarget::File(path) => vec![
                        Action::LaunchApplication {
                            path: path.clone(),
                            usage_key: entry.key.clone(),
                        },
                        Action::RunAsAdministrator { path: path.clone() },
                        Action::OpenContainingFolder { path: path.clone() },
                        Action::CopyText {
                            text: path.to_string_lossy().into_owned(),
                        },
                    ],
                    EntryTarget::Packaged(app_id) => vec![
                        Action::LaunchPackagedApplication {
                            app_id: app_id.clone(),
                            usage_key: entry.key.clone(),
                        },
                        Action::CopyText {
                            text: app_id.clone(),
                        },
                    ],
                };
                Some(ResultItem {
                    provider_id: PROVIDER_ID,
                    title: entry.name.clone(),
                    score,
                    actions,
                })
            })
            .collect();
        ProviderOutput {
            results,
            exclusive: false,
        }
    }
}

fn collect_dir(
    dir: &std::path::Path,
    out: &mut Vec<Entry>,
    blocked: &[String],
    allowed: &[String],
    depth: u8,
) -> bool {
    if depth == 0 {
        return true;
    }
    let Ok(items) = fs::read_dir(dir) else {
        return false;
    };
    for item in items.flatten() {
        let path = item.path();
        if path.is_dir() {
            collect_dir(&path, out, blocked, allowed, depth - 1);
            continue;
        }
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "lnk" | "exe" | "url" | "appref-ms") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .trim()
            .to_owned();
        let explicitly_allowed = matches_patterns(&name, allowed);
        if !name.is_empty()
            && (explicitly_allowed
                || (!is_uninstaller(&name)
                    && !is_blocked_tool(&name)
                    && !is_system_setting(&name)
                    && !is_noise_entry(&name, &path)
                    && !is_user_blocked(&name, blocked)))
        {
            let (pinyin, initials) = search_keys(&name);
            out.push(Entry {
                key: name.to_lowercase(),
                pinyin,
                initials,
                name,
                target: EntryTarget::File(path),
            });
        }
    }
    true
}

fn collect_packaged_apps(out: &mut Vec<Entry>, blocked: &[String], allowed: &[String]) -> bool {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = r#"[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; Get-StartApps | ForEach-Object { "$($_.Name)`t$($_.AppID)" }"#;
    let Some(powershell) = system_program(r"System32\WindowsPowerShell\v1.0\powershell.exe") else {
        return false;
    };
    let child = Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return false;
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        return false;
    };
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let Ok(Ok(bytes)) = reader.join() else {
        return false;
    };
    if !status.is_some_and(|status| status.success()) {
        return false;
    }
    let text = String::from_utf8_lossy(&bytes);
    for (name, app_id) in parse_start_apps(&text) {
        let explicitly_allowed = matches_patterns(&name, allowed);
        if !explicitly_allowed
            && (is_uninstaller(&name) || is_blocked_tool(&name) || is_user_blocked(&name, blocked))
        {
            continue;
        }
        let (pinyin, initials) = search_keys(&name);
        out.push(Entry {
            key: name.to_lowercase(),
            pinyin,
            initials,
            name,
            target: EntryTarget::Packaged(app_id),
        });
    }
    true
}

fn system_program(relative: &str) -> Option<PathBuf> {
    let root = PathBuf::from(env::var_os("SystemRoot")?);
    root.is_absolute()
        .then(|| root.join(relative))
        .filter(|path| path.is_file())
}

pub(crate) fn parse_start_apps(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let (name, app_id) = line.trim_end_matches('\r').split_once('\t')?;
            let name = name.trim().trim_start_matches('\u{feff}');
            let app_id = app_id.trim();
            (!name.is_empty() && app_id.contains('!')).then(|| (name.to_owned(), app_id.to_owned()))
        })
        .collect()
}

pub(crate) fn entry_score(
    entry: &Entry,
    query: &str,
    usage: crate::model::Usage,
    now: u64,
) -> Option<i32> {
    rank_entry(
        &entry.key,
        &entry.pinyin,
        &entry.initials,
        query,
        usage,
        now,
    )
}

pub(crate) fn is_uninstaller(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("卸载")
        || lower.contains("移除")
        || lower.contains("uninstall")
        || lower.contains("uninst")
        || lower.contains("unins")
}

pub(crate) fn is_blocked_tool(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    let phrases = [
        "powershell",
        "command prompt",
        "命令提示符",
        "windows terminal",
        "python",
        "idle",
        "anaconda",
        "miniconda",
        "conda prompt",
        "jupyter",
        "git bash",
        "git cmd",
        "git gui",
        "github desktop",
        "visual studio",
        "visual studio code",
        "android studio",
        "intellij idea",
        "pycharm",
        "webstorm",
        "clion",
        "rider",
        "goland",
        "phpstorm",
        "rubymine",
        "eclipse",
        "netbeans",
        "node.js",
        "nodejs",
        "docker desktop",
        "kubernetes",
        "rustup",
        "wsl",
        "ubuntu",
        "debian",
        "kali linux",
        "developer command prompt",
        "developer powershell",
    ];
    let exact = [
        "cmd", "cmd.exe", "git", "node", "npm", "yarn", "pnpm", "bun", "cargo", "rust", "rustc",
        "go", "golang", "java", "javac", "jdk", "docker", "kubectl", "terminal",
    ];
    phrases.iter().any(|phrase| lower.contains(phrase))
        || exact.contains(&lower.as_str())
        || lower.starts_with("git ")
        || lower.starts_with("java ")
        || lower.starts_with("jdk ")
        || lower.starts_with("node ")
        || lower.starts_with("rust ")
}

pub(crate) fn is_system_setting(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    let phrases = [
        "control panel",
        "控制面板",
        "windows settings",
        "系统设置",
        "windows tools",
        "windows 工具",
        "administrative tools",
        "管理工具",
        "computer management",
        "计算机管理",
        "disk management",
        "磁盘管理",
        "disk cleanup",
        "磁盘清理",
        "defragment",
        "碎片整理",
        "optimize drives",
        "优化驱动器",
        "event viewer",
        "事件查看器",
        "task scheduler",
        "任务计划程序",
        "registry editor",
        "注册表编辑器",
        "system configuration",
        "系统配置",
        "component services",
        "组件服务",
        "local security policy",
        "本地安全策略",
        "group policy",
        "组策略",
        "windows defender firewall with advanced security",
        "高级安全 windows defender 防火墙",
        "print management",
        "打印管理",
        "odbc data sources",
        "odbc 数据源",
        "iscsi initiator",
        "iscsi 发起程序",
        "recovery drive",
        "恢复驱动器",
        "performance monitor",
        "性能监视器",
        "memory diagnostic",
        "内存诊断",
        "system information",
        "系统信息",
        "system properties",
        "系统属性",
    ];
    phrases.iter().any(|phrase| lower.contains(phrase))
        || matches!(lower.as_str(), "settings" | "设置" | "services" | "服务")
}

pub(crate) fn is_noise_entry(name: &str, path: &std::path::Path) -> bool {
    let name = name.trim().to_lowercase();
    let path = path.to_string_lossy().to_lowercase();
    let noisy_names = [
        "application verifier",
        "tools for desktop apps",
        "windows sdk",
        "windows kits",
        "debugging tools",
        "developer tools",
        "release notes",
        "readme",
        "documentation",
        "command line tools",
        "sdk manager",
    ];
    let noisy_paths = [
        "windows kits",
        "microsoft sdks",
        "visual studio tools",
        "administrative tools",
        "windows\\start menu\\programs\\windows tools",
    ];
    noisy_names.iter().any(|value| name.contains(value))
        || noisy_paths.iter().any(|value| path.contains(value))
        || name.ends_with(" help")
        || name.ends_with(" manual")
}

pub(crate) fn is_user_blocked(name: &str, blocked: &[String]) -> bool {
    let lower = name.to_lowercase();
    blocked.iter().any(|pattern| lower.contains(pattern))
}

fn matches_patterns(name: &str, patterns: &[String]) -> bool {
    let lower = name.to_lowercase();
    patterns.iter().any(|pattern| lower.contains(pattern))
}
