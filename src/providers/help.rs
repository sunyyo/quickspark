use crate::model::{Action, Query, ResultItem};

use super::{Provider, ProviderContext, ProviderOutput};

const PROVIDER_ID: &str = "help";

pub(crate) struct HelpProvider;

impl Provider for HelpProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, _context: &ProviderContext<'_>) -> ProviderOutput {
        let Some(terms) = help_query(query.raw()) else {
            return ProviderOutput::empty();
        };
        let results = TOPICS
            .iter()
            .filter(|topic| topic_matches(topic, terms))
            .enumerate()
            .map(|(position, topic)| topic_result(topic, position))
            .collect::<Vec<_>>();

        if results.is_empty() {
            return ProviderOutput {
                results: vec![ResultItem {
                    provider_id: PROVIDER_ID,
                    title: format!("没有找到“{terms}”的说明；输入 ? 查看全部功能"),
                    score: 100_000,
                    actions: vec![Action::SetQuery {
                        query: "?".to_owned(),
                    }],
                }],
                exclusive: true,
            };
        }

        ProviderOutput {
            results,
            exclusive: true,
        }
    }
}

struct HelpTopic {
    title: &'static str,
    details: &'static str,
    keywords: &'static str,
    example: Option<&'static str>,
}

const TOPICS: &[HelpTopic] = &[
    HelpTopic {
        title: "使用说明：输入 ? 关键词筛选；F1 随时打开说明书",
        details: "输入 ?、help、帮助或说明书可查看全部说明；也可以继续输入关键词筛选。",
        keywords: "帮助 说明书 手册 manual help f1 搜索说明",
        example: Some("? 文件"),
    },
    HelpTopic {
        title: "应用搜索：直接输入名称、拼音或拼音首字母",
        details: "搜索开始菜单、桌面快捷方式和系统应用，并按相关性与使用频率排序。",
        keywords: "应用 程序 软件 拼音 首字母 模糊搜索 启动",
        example: Some("obsidian"),
    },
    HelpTopic {
        title: "结果操作：↑↓选择；Enter 执行；Tab 查看更多动作",
        details: "Alt+1 至 Alt+8 可直接执行对应结果；Esc 隐藏启动器。",
        keywords: "结果 操作 快捷键 上下 选择 enter tab alt esc 执行",
        example: None,
    },
    HelpTopic {
        title: "快捷动作：管理员运行、打开位置和复制",
        details: "Ctrl+Enter 以管理员身份运行；Ctrl+Shift+E 打开所在位置；Ctrl+C 复制。",
        keywords: "快捷动作 快捷键 管理员 ctrl enter 打开位置 定位 shift 复制",
        example: None,
    },
    HelpTopic {
        title: "设置中心：settings / set / 设置",
        details: "可切换唤起快捷键、开机启动、主题、结果数量和默认网页搜索引擎。",
        keywords: "设置 中心 settings set 快捷键 开机启动 主题 结果 搜索引擎",
        example: Some("settings"),
    },
    HelpTopic {
        title: "自定义指令：; 查看；; edit 编辑配置",
        details: "在 commands.txt 中按“别名 | 标题 | 目标”添加目录、网址或 URI 指令。",
        keywords: "自定义 指令 命令 commands 分号 别名 配置",
        example: Some("; edit"),
    },
    HelpTopic {
        title: "直接打开网址：http://、https:// 或 www.",
        details: "粘贴完整 HTTP/HTTPS 网址，或输入 www. 开头的域名，按 Enter 使用默认浏览器打开。",
        keywords: "网址 网页 浏览器 url http https www 打开",
        example: Some("www.rust-lang.org"),
    },
    HelpTopic {
        title: "AI 问答：ai 自动判断联网；--web 强制；--no-web 禁用",
        details: "模型可使用 Tavily 进行最多三轮查证，来源仅作为内部证据；输入 ai settings 配置模型和两类 API Key。",
        keywords: "ai 人工智能 大模型 问答 tavily 联网 搜索 查证 来源 api key openai deepseek",
        example: Some("ai Rust 最新稳定版有哪些变化"),
    },
    HelpTopic {
        title: "AI 翻译：tr 文本；tr --to en / zh / ja / ko 文本",
        details: "由 AI 识别原文语言：中文译成英语，任何非中文语言译成简体中文；译文优先采用自然、地道的母语表达。",
        keywords: "ai 翻译 translate translation tr fy 中英互译 中文 英语 日语 韩语",
        example: Some("tr 你好，世界"),
    },
    HelpTopic {
        title: "收藏置顶：选中结果后按 Tab，选择固定到顶部",
        details: "收藏保存在本机 pinned.txt；再次打开动作菜单可以取消固定。",
        keywords: "收藏 固定 置顶 pinned favorite tab 取消",
        example: None,
    },
    HelpTopic {
        title: "系统命令：sys；Windows 设置搜索：$ 关键词",
        details: "支持锁定、睡眠、注销、关机、重启、回收站以及常用 Windows 设置页面。",
        keywords: "系统 命令 sys windows 设置 美元 锁定 睡眠 关机 重启",
        example: Some("sys"),
    },
    HelpTopic {
        title: "文件搜索：f 关键词（使用 Everything 索引）",
        details: "支持 Everything 语法，例如 *.pdf、ext:rs 和 path:；Tab 可打开位置或复制路径。",
        keywords: "文件 文件夹 everything 搜索 f file 通配符 扩展名 路径",
        example: Some("f *.pdf"),
    },
    HelpTopic {
        title: "窗口切换：win 或 win 窗口标题",
        details: "输入 win 列出当前窗口，输入 win 关键词可按窗口标题筛选。",
        keywords: "窗口 切换 win window 标题",
        example: Some("win"),
    },
    HelpTopic {
        title: "网页搜索：ss / bd / bing / gg + 关键词",
        details: "ss 同时列出百度、必应和 Google；其他前缀直接选择对应搜索引擎。",
        keywords: "网页 网络 搜索 百度 必应 google 浏览器 ss bd bing gg",
        example: Some("ss 天气"),
    },
    HelpTopic {
        title: "计算器：= 数学表达式",
        details: "支持加减乘除、取余、括号、小数和一元正负号；Enter 或 Ctrl+C 复制结果。",
        keywords: "计算器 计算 数学 表达式 加减乘除 取余 括号 等号",
        example: Some("= 12 * (8 + 3)"),
    },
    HelpTopic {
        title: "单位换算：%% 数值 单位 to 单位",
        details: "支持长度、重量、时间、速度、存储容量和摄氏/华氏/开尔文温度。",
        keywords: "单位 换算 转换 长度 重量 时间 速度 容量 温度 to",
        example: Some("%% 10 km to mi"),
    },
    HelpTopic {
        title: "日期计算：date 日期 +30d / -2w",
        details: "日期可以使用 YYYY-MM-DD 或 today；偏移量支持天 d 和周 w。",
        keywords: "日期 时间 计算 date today 天 周 d w 加减",
        example: Some("date today + 30d"),
    },
    HelpTopic {
        title: "进制转换：hex / bin / oct / dec + 数值",
        details: "支持二进制、八进制、十进制和十六进制之间的转换。",
        keywords: "进制 转换 二进制 八进制 十进制 十六进制 hex bin oct dec",
        example: Some("hex 255"),
    },
    HelpTopic {
        title: "个性化：托盘切换主题、刷新索引或退出",
        details: "右键托盘图标可显示启动器、打开说明、刷新应用索引、切换亮暗主题和退出。",
        keywords: "个性化 托盘 主题 亮色 暗色 刷新 索引 退出 设置",
        example: None,
    },
];

fn help_query(input: &str) -> Option<&str> {
    let input = input.trim();
    if let Some(terms) = input.strip_prefix('?').or_else(|| input.strip_prefix('？')) {
        return Some(terms.trim());
    }
    if ["help", "manual", "帮助", "说明", "说明书"]
        .iter()
        .any(|prefix| input.eq_ignore_ascii_case(prefix))
    {
        return Some("");
    }
    let split_at = input.find(char::is_whitespace)?;
    let (prefix, terms) = input.split_at(split_at);
    ["help", "manual", "帮助", "说明", "说明书"]
        .iter()
        .any(|candidate| prefix.eq_ignore_ascii_case(candidate))
        .then_some(terms.trim())
}

fn topic_matches(topic: &HelpTopic, terms: &str) -> bool {
    if terms.is_empty() {
        return true;
    }
    let searchable = format!("{} {} {}", topic.title, topic.details, topic.keywords).to_lowercase();
    terms
        .to_lowercase()
        .split_whitespace()
        .all(|term| searchable.contains(term))
}

fn topic_result(topic: &HelpTopic, position: usize) -> ResultItem {
    let mut actions = Vec::with_capacity(2);
    if let Some(example) = topic.example {
        actions.push(Action::SetQuery {
            query: example.to_owned(),
        });
    }
    actions.push(Action::CopyText {
        text: topic.example.map_or_else(
            || format!("{}\n{}", topic.title, topic.details),
            |example| format!("{}\n{}\n示例：{example}", topic.title, topic.details),
        ),
    });
    ResultItem {
        provider_id: PROVIDER_ID,
        title: topic.title.to_owned(),
        score: 100_000 - position as i32,
        actions,
    }
}

#[cfg(test)]
mod tests {
    use super::{HelpProvider, TOPICS, help_query};
    use crate::storage::Settings;
    use crate::{
        model::{Action, Query},
        providers::{Provider, ProviderContext},
    };
    use std::collections::{HashMap, HashSet};

    #[test]
    fn recognizes_help_commands_without_stealing_regular_searches() {
        assert_eq!(help_query("?"), Some(""));
        assert_eq!(help_query("? 文件"), Some("文件"));
        assert_eq!(help_query("？计算器"), Some("计算器"));
        assert_eq!(help_query("HELP 快捷键"), Some("快捷键"));
        assert_eq!(help_query("说明书"), Some(""));
        assert_eq!(help_query("helper"), None);
        assert_eq!(help_query("说明文件"), None);
    }

    #[test]
    fn lists_and_filters_manual_topics() {
        let provider = HelpProvider;
        let usage = HashMap::new();
        let pinned = HashSet::new();
        let settings = Settings::default();
        let context = ProviderContext {
            usage: &usage,
            now: 0,
            settings: &settings,
            pinned: &pinned,
            light_theme: false,
        };
        let all = provider.query(&Query::new("?"), &context);
        assert!(all.exclusive);
        assert_eq!(all.results.len(), TOPICS.len());

        let files = provider.query(&Query::new("? 文件"), &context);
        assert!(files.exclusive);
        assert!(
            files
                .results
                .iter()
                .any(|item| item.title.contains("文件搜索"))
        );
        assert!(matches!(
            files.results[0].actions.first(),
            Some(Action::SetQuery { query }) if query == "f *.pdf"
        ));

        let ai = TOPICS
            .iter()
            .find(|topic| topic.title.starts_with("AI 问答"))
            .expect("AI help topic should exist");
        assert!(ai.details.contains("来源仅作为内部证据"));
        assert!(!ai.details.contains("完成后附来源"));
    }
}
