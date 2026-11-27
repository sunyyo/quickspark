use crate::{
    model::{Action, Query, ResultItem},
    storage::WebEngine,
};

use super::{Provider, ProviderContext, ProviderOutput, split_first_term};

const PROVIDER_ID: &str = "web-search";

pub(crate) struct WebSearchProvider;

impl Provider for WebSearchProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, context: &ProviderContext<'_>) -> ProviderOutput {
        let Some((prefix, terms)) = parse_web_prefix(query.raw()) else {
            return ProviderOutput::empty();
        };
        if terms.is_empty() {
            return ProviderOutput::exclusive_item(
                PROVIDER_ID,
                "网页搜索：在 ss / web / bd / bing / gg 后输入关键词",
                100_000,
                Vec::new(),
            );
        }
        let results = build_web_searches(prefix, terms, context.settings.web_engine)
            .into_iter()
            .enumerate()
            .map(|(position, search)| ResultItem {
                provider_id: PROVIDER_ID,
                title: search.label,
                score: 100_000 - position as i32,
                actions: vec![
                    Action::OpenUrl {
                        url: search.url.clone(),
                    },
                    Action::CopyText { text: search.url },
                ],
            })
            .collect();
        ProviderOutput::exclusive(results)
    }
}

#[derive(Clone, Copy)]
enum WebPrefix {
    Combined,
    Default,
    Engine(WebEngine),
}

fn parse_web_prefix(input: &str) -> Option<(WebPrefix, &str)> {
    let (head, terms) = split_first_term(input);
    let prefix = match head.to_ascii_lowercase().as_str() {
        "ss" => WebPrefix::Combined,
        "web" => WebPrefix::Default,
        "bd" => WebPrefix::Engine(WebEngine::Baidu),
        "bing" => WebPrefix::Engine(WebEngine::Bing),
        "gg" => WebPrefix::Engine(WebEngine::Google),
        _ => return None,
    };
    Some((prefix, terms))
}

#[cfg(test)]
fn is_empty_web_prefix(input: &str) -> bool {
    matches!(parse_web_prefix(input), Some((_, "")))
}

pub(crate) struct WebSearch {
    pub(crate) label: String,
    pub(crate) url: String,
}

fn engine_search(engine: WebEngine, terms: &str) -> WebSearch {
    let base = match engine {
        WebEngine::Baidu => "https://www.baidu.com/s?wd=",
        WebEngine::Bing => "https://www.bing.com/search?q=",
        WebEngine::Google => "https://www.google.com/search?q=",
    };
    WebSearch {
        label: format!("使用{}搜索“{terms}”", engine.label()),
        url: format!("{base}{}", url_encode(terms)),
    }
}

fn build_web_searches(prefix: WebPrefix, terms: &str, default_engine: WebEngine) -> Vec<WebSearch> {
    match prefix {
        WebPrefix::Combined => std::iter::once(default_engine)
            .chain(
                [WebEngine::Baidu, WebEngine::Bing, WebEngine::Google]
                    .into_iter()
                    .filter(|engine| *engine != default_engine),
            )
            .map(|engine| engine_search(engine, terms))
            .collect(),
        WebPrefix::Default => vec![engine_search(default_engine, terms)],
        WebPrefix::Engine(engine) => vec![engine_search(engine, terms)],
    }
}

#[cfg(test)]
pub(crate) fn parse_web_searches(input: &str) -> Option<Vec<WebSearch>> {
    parse_web_searches_with_engine(input, WebEngine::Baidu)
}

#[cfg(test)]
fn parse_web_searches_with_engine(
    input: &str,
    default_engine: WebEngine,
) -> Option<Vec<WebSearch>> {
    let (prefix, terms) = parse_web_prefix(input)?;
    (!terms.is_empty()).then(|| build_web_searches(prefix, terms, default_engine))
}

pub(crate) fn url_encode(text: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{is_empty_web_prefix, parse_web_searches_with_engine};
    use crate::storage::WebEngine;

    #[test]
    fn configurable_engine_leads_combined_search_and_handles_web_prefix() {
        let combined = parse_web_searches_with_engine("ss rust", WebEngine::Google).unwrap();
        assert!(combined[0].label.starts_with("使用Google"));
        let default = parse_web_searches_with_engine("web rust", WebEngine::Bing).unwrap();
        assert_eq!(default.len(), 1);
        assert!(default[0].label.starts_with("使用必应"));
    }

    #[test]
    fn empty_prefixes_match_supported_commands_without_stealing_app_names() {
        for prefix in ["ss", "web", "bd", "bing", "gg", "WEB"] {
            assert!(is_empty_web_prefix(prefix), "{prefix} should match");
        }
        for app_query in ["google", "baidu", "google chrome"] {
            assert!(
                !is_empty_web_prefix(app_query),
                "{app_query} should not be hijacked"
            );
        }
    }
}
