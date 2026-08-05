use std::collections::HashSet;

use serde_json::{Value, json};

use super::transport::{RequestContext, winhttp_json};

const ENDPOINT: &str = "https://api.tavily.com/search";
const MAX_QUERY_CHARS: usize = 400;
const MAX_SOURCE_CONTENT_CHARS: usize = 1_800;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SearchSource {
    pub(super) title: String,
    pub(super) url: String,
    pub(super) content: String,
}

pub(super) fn search(
    query: &str,
    api_key: &str,
    timeout_seconds: u32,
    context: RequestContext,
) -> Result<Vec<SearchSource>, String> {
    if api_key.trim().is_empty() {
        return Err(
            "模型判断需要联网，但尚未设置 Tavily API Key；请输入 ai tavily key 进行安全设置"
                .to_owned(),
        );
    }
    let query = truncate_chars(query.trim(), MAX_QUERY_CHARS);
    if query.is_empty() {
        return Err("模型生成了空的网络搜索词".to_owned());
    }
    let body = search_body(&query);
    let response = winhttp_json(ENDPOINT, api_key, &body, timeout_seconds, context, "Tavily")?;
    extract_sources(&response)
}

fn search_body(query: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "query": query,
        "search_depth": "basic",
        "max_results": 5,
        "topic": "general",
        "include_answer": false,
        "include_raw_content": false,
        "include_images": false,
        "auto_parameters": false
    }))
    .unwrap_or_default()
}

pub(super) fn extract_sources(bytes: &[u8]) -> Result<Vec<SearchSource>, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Tavily 返回的不是有效 JSON：{error}"))?;
    if let Some(message) = value
        .get("error")
        .and_then(|error| error.get("message").or(Some(error)))
        .and_then(Value::as_str)
    {
        return Err(message.to_owned());
    }
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "Tavily 响应中缺少 results".to_owned())?;
    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for result in results {
        let Some(url) = result.get("url").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if !(url.starts_with("https://") || url.starts_with("http://"))
            || url.chars().count() > 2_048
            || url.chars().any(char::is_whitespace)
        {
            continue;
        }
        let normalized_url = url.to_ascii_lowercase();
        if !seen.insert(normalized_url) {
            continue;
        }
        let title = result
            .get("title")
            .and_then(Value::as_str)
            .map(flatten)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| url.to_owned());
        let content = result
            .get("content")
            .and_then(Value::as_str)
            .map(flatten)
            .unwrap_or_default();
        sources.push(SearchSource {
            title: truncate_chars(&title, 240),
            url: url.to_owned(),
            content: truncate_chars(&content, MAX_SOURCE_CONTENT_CHARS),
        });
    }
    Ok(sources)
}

fn flatten(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let mut output = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        output.push('…');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{SearchSource, extract_sources, search_body};

    #[test]
    fn basic_search_body_omits_advanced_only_chunk_setting() {
        let body: serde_json::Value = serde_json::from_slice(&search_body("rust")).unwrap();
        assert_eq!(body["search_depth"], "basic");
        assert_eq!(body["max_results"], 5);
        assert!(body.get("chunks_per_source").is_none());
    }

    #[test]
    fn parses_deduplicates_and_filters_tavily_sources() {
        let response = br#"{
            "results": [
                {
                    "title": " Rust  Releases ",
                    "url": "https://www.rust-lang.org/releases",
                    "content": "first\n  second"
                },
                {
                    "title": "duplicate",
                    "url": "HTTPS://WWW.RUST-LANG.ORG/releases",
                    "content": "ignored"
                },
                {
                    "title": "unsafe",
                    "url": "javascript:alert(1)",
                    "content": "ignored"
                }
            ]
        }"#;
        assert_eq!(
            extract_sources(response).unwrap(),
            vec![SearchSource {
                title: "Rust Releases".to_owned(),
                url: "https://www.rust-lang.org/releases".to_owned(),
                content: "first second".to_owned(),
            }]
        );
        assert!(extract_sources(br#"{"answer":"missing"}"#).is_err());
    }
}
