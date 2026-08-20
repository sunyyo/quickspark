use std::{env, fs};

use crate::{
    model::{Action, Query, ResultItem},
    storage::commands_file,
};

use super::{Provider, ProviderContext, ProviderOutput, web_search::url_encode};

const PROVIDER_ID: &str = "commands";

pub(crate) struct CommandProvider;

impl Provider for CommandProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, _context: &ProviderContext<'_>) -> ProviderOutput {
        let Some(terms) = commands_query(query.raw()) else {
            return ProviderOutput::empty();
        };
        let Some(path) = commands_file() else {
            return message("无法创建自定义指令配置文件", Vec::new());
        };
        if terms.eq_ignore_ascii_case("edit") || terms == "编辑" {
            return message(
                "打开 commands.txt 编辑自定义指令",
                vec![Action::OpenPath { path }],
            );
        }

        let commands = fs::read_to_string(&path)
            .ok()
            .map(|text| parse_commands(&text))
            .unwrap_or_default();
        if commands.is_empty() {
            return message(
                "尚未添加自定义指令；按 Enter 打开 commands.txt",
                vec![Action::OpenPath { path }],
            );
        }

        let (alias, arguments) = split_command_terms(terms);
        let exact = commands
            .iter()
            .any(|command| command.alias.eq_ignore_ascii_case(alias));
        let results = commands
            .iter()
            .filter(|command| {
                if terms.is_empty() {
                    true
                } else if exact {
                    command.alias.eq_ignore_ascii_case(alias)
                } else {
                    let searchable = format!("{} {}", command.alias, command.title).to_lowercase();
                    searchable.contains(&terms.to_lowercase())
                }
            })
            .enumerate()
            .map(|(position, command)| {
                let action = command_action(command, arguments);
                ResultItem {
                    provider_id: PROVIDER_ID,
                    title: format!("{}：{}", command.alias, command.title),
                    score: 100_000 - position as i32,
                    actions: vec![
                        action,
                        Action::CopyText {
                            text: command.target.clone(),
                        },
                    ],
                }
            })
            .collect::<Vec<_>>();
        if results.is_empty() {
            return message(
                &format!("没有找到“{terms}”指令；输入 ; edit 编辑配置"),
                vec![Action::OpenPath { path }],
            );
        }
        ProviderOutput {
            results,
            exclusive: true,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CustomCommand {
    alias: String,
    title: String,
    target: String,
}

fn parse_commands(text: &str) -> Vec<CustomCommand> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut fields = line.splitn(3, '|').map(str::trim);
            let (alias, title, target) = (fields.next()?, fields.next()?, fields.next()?);
            if alias.is_empty()
                || alias.len() > 32
                || alias.chars().any(char::is_whitespace)
                || alias.contains(';')
                || title.is_empty()
                || target.is_empty()
            {
                return None;
            }
            Some(CustomCommand {
                alias: alias.to_owned(),
                title: title.to_owned(),
                target: target.to_owned(),
            })
        })
        .collect()
}

fn commands_query(input: &str) -> Option<&str> {
    let input = input.trim();
    input.strip_prefix(';').map(str::trim)
}

fn split_command_terms(terms: &str) -> (&str, &str) {
    terms
        .split_once(char::is_whitespace)
        .map_or((terms, ""), |(alias, arguments)| (alias, arguments.trim()))
}

fn command_action(command: &CustomCommand, arguments: &str) -> Action {
    let mut target = expand_environment(&command.target);
    let replacement = if target.trim_start_matches("url:").starts_with("http://")
        || target.trim_start_matches("url:").starts_with("https://")
    {
        url_encode(arguments)
    } else {
        arguments.to_owned()
    };
    target = target.replace("{query}", &replacement);
    let target = target
        .strip_prefix("url:")
        .or_else(|| target.strip_prefix("path:"))
        .unwrap_or(&target)
        .trim()
        .to_owned();
    if target.starts_with("http://") || target.starts_with("https://") {
        Action::OpenUrl { url: target }
    } else {
        Action::OpenTarget { target }
    }
}

fn expand_environment(value: &str) -> String {
    let mut expanded = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find('%') {
        expanded.push_str(&remaining[..start]);
        let after_start = &remaining[start + 1..];
        let Some(end) = after_start.find('%') else {
            expanded.push_str(&remaining[start..]);
            return expanded;
        };
        let name = &after_start[..end];
        if !name.is_empty()
            && let Ok(replacement) = env::var(name)
        {
            expanded.push_str(&replacement);
        } else {
            expanded.push_str(&remaining[start..start + end + 2]);
        }
        remaining = &after_start[end + 1..];
    }
    expanded.push_str(remaining);
    expanded
}

fn message(title: &str, actions: Vec<Action>) -> ProviderOutput {
    ProviderOutput {
        results: vec![ResultItem {
            provider_id: PROVIDER_ID,
            title: title.to_owned(),
            score: 100_000,
            actions,
        }],
        exclusive: true,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CustomCommand, command_action, commands_query, expand_environment, parse_commands,
    };
    use crate::model::Action;

    #[test]
    fn parses_valid_command_rows_and_skips_invalid_ones() {
        let commands = parse_commands(
            "# comment\ndocs | 文档 | %USERPROFILE%\\Documents\ngh | GitHub | https://github.com/search?q={query}\nbad row\nbad alias | title | target\n",
        );
        assert_eq!(
            commands,
            vec![
                CustomCommand {
                    alias: "docs".to_owned(),
                    title: "文档".to_owned(),
                    target: r"%USERPROFILE%\Documents".to_owned(),
                },
                CustomCommand {
                    alias: "gh".to_owned(),
                    title: "GitHub".to_owned(),
                    target: "https://github.com/search?q={query}".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn recognizes_explicit_command_prefixes() {
        assert_eq!(commands_query(";"), Some(""));
        assert_eq!(commands_query("; gh rust"), Some("gh rust"));
        assert_eq!(commands_query("; edit"), Some("edit"));
        assert_eq!(commands_query("cmd"), None);
        assert_eq!(commands_query("cmd edit"), None);
        assert_eq!(commands_query("cmd.exe"), None);
    }

    #[test]
    fn expands_url_query_placeholders_safely() {
        let command = CustomCommand {
            alias: "gh".to_owned(),
            title: "GitHub".to_owned(),
            target: "https://github.com/search?q={query}".to_owned(),
        };
        assert!(matches!(
            command_action(&command, "rust 语言"),
            Action::OpenUrl { url } if url.ends_with("rust%20%E8%AF%AD%E8%A8%80")
        ));
    }

    #[test]
    fn expands_windows_environment_names_case_insensitively() {
        let system_root = std::env::var("SystemRoot").unwrap();
        assert_eq!(expand_environment("%systemroot%"), system_root);
        assert_eq!(
            expand_environment("%QUICKSPARK_MISSING_VARIABLE%"),
            "%QUICKSPARK_MISSING_VARIABLE%"
        );
    }
}
