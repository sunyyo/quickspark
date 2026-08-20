use crate::{
    model::{Action, Query, ResultItem},
    storage::WebEngine,
};

use super::{Provider, ProviderContext, ProviderOutput};

const PROVIDER_ID: &str = "web-search";

pub(crate) struct WebSearchProvider;

impl Provider for WebSearchProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, context: &ProviderContext<'_>) -> ProviderOutput {
        if is_empty_web_prefix(query.raw()) {
            return ProviderOutput {
                results: vec![ResultItem {
                    provider_id: PROVIDER_ID,
                    title: "网页搜索：在 ss / bd / bing / gg 后输入关键词".to_owned(),
                    score: 100_000,
                    actions: Vec::new(),
                }],
                exclusive: true,
            };
        }
        let Some(searches) =
            parse_web_searches_with_engine(query.raw(), context.settings.web_engine)
        else {
            return ProviderOutput::empty();
        };
        let results = searches
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
        ProviderOutput {
            results,
            exclusive: true,
        }
    }
}

fn is_empty_web_prefix(input: &str) -> bool {
    ["ss", "bd", "baidu", "bing", "gg", "google"]
        .iter()
        .any(|prefix| input.trim().eq_ignore_ascii_case(prefix))
}

pub(crate) struct WebSearch {
    pub(crate) label: String,
    pub(crate) url: String,
}

fn web_search(engine: &str, base: &str, terms: &str) -> WebSearch {
    WebSearch {
        label: format!("使用{engine}搜索“{terms}”"),
        url: format!("{base}{}", url_encode(terms)),
    }
}

fn engine_search(engine: WebEngine, terms: &str) -> WebSearch {
    match engine {
        WebEngine::Baidu => web_search("百度", "https://www.baidu.com/s?wd=", terms),
        WebEngine::Bing => web_search("必应", "https://www.bing.com/search?q=", terms),
        WebEngine::Google => web_search("Google", "https://www.google.com/search?q=", terms),
    }
}

#[cfg(test)]
pub(crate) fn parse_web_searches(input: &str) -> Option<Vec<WebSearch>> {
    parse_web_searches_with_engine(input, WebEngine::Baidu)
}

fn parse_web_searches_with_engine(
    input: &str,
    default_engine: WebEngine,
) -> Option<Vec<WebSearch>> {
    let trimmed = input.trim();
    let split = trimmed.find(char::is_whitespace)?;
    let prefix = trimmed[..split].to_ascii_lowercase();
    let terms = trimmed[split..].trim();
    if terms.is_empty() {
        return None;
    }
    let searches = match prefix.as_str() {
        "ss" => std::iter::once(default_engine)
            .chain(
                [WebEngine::Baidu, WebEngine::Bing, WebEngine::Google]
                    .into_iter()
                    .filter(|engine| *engine != default_engine),
            )
            .map(|engine| engine_search(engine, terms))
            .collect(),
        "web" => vec![engine_search(default_engine, terms)],
        "bd" => vec![engine_search(WebEngine::Baidu, terms)],
        "bing" => vec![engine_search(WebEngine::Bing, terms)],
        "gg" => vec![engine_search(WebEngine::Google, terms)],
        _ => return None,
    };
    Some(searches)
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
    use super::parse_web_searches_with_engine;
    use crate::storage::WebEngine;

    #[test]
    fn configurable_engine_leads_combined_search_and_handles_web_prefix() {
        let combined = parse_web_searches_with_engine("ss rust", WebEngine::Google).unwrap();
        assert!(combined[0].label.starts_with("使用Google"));
        let default = parse_web_searches_with_engine("web rust", WebEngine::Bing).unwrap();
        assert_eq!(default.len(), 1);
        assert!(default[0].label.starts_with("使用必应"));
    }
}
