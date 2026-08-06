use std::net::{IpAddr, Ipv6Addr};

use crate::model::{Action, Query, ResultItem};

use super::{Provider, ProviderContext, ProviderOutput};

const MAX_URL_CHARS: usize = 8_192;

pub(crate) struct UrlProvider;

impl Provider for UrlProvider {
    fn id(&self) -> &'static str {
        "url"
    }

    fn query(&self, query: &Query, _context: &ProviderContext<'_>) -> ProviderOutput {
        let Some(url) = direct_url(query.raw()) else {
            return ProviderOutput::empty();
        };

        let title = format!("使用默认浏览器打开：{}", preview(&url));
        ProviderOutput {
            results: vec![ResultItem {
                provider_id: self.id(),
                title,
                score: 100_000,
                actions: vec![
                    Action::OpenUrl { url: url.clone() },
                    Action::CopyText { text: url },
                ],
            }],
            exclusive: true,
        }
    }
}

fn direct_url(raw: &str) -> Option<String> {
    let input = raw.trim();
    if input.is_empty()
        || input.chars().count() > MAX_URL_CHARS
        || input
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return None;
    }

    let (url, authority_start, shorthand) =
        if let Some(rest) = strip_prefix_ascii_case(input, "https://") {
            (format!("https://{rest}"), "https://".len(), false)
        } else if let Some(rest) = strip_prefix_ascii_case(input, "http://") {
            (format!("http://{rest}"), "http://".len(), false)
        } else if strip_prefix_ascii_case(input, "www.").is_some() {
            (format!("https://{input}"), "https://".len(), true)
        } else {
            return None;
        };

    let remainder = &url[authority_start..];
    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];

    if shorthand && authority.get(4..).is_none_or(str::is_empty) {
        return None;
    }

    valid_authority(authority).then_some(url)
}

fn strip_prefix_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn valid_authority(authority: &str) -> bool {
    if authority.is_empty() || authority.contains('@') {
        return false;
    }

    if let Some(bracketed) = authority.strip_prefix('[') {
        let Some(close) = bracketed.find(']') else {
            return false;
        };
        let host = &bracketed[..close];
        let suffix = &bracketed[close + 1..];
        return host.parse::<Ipv6Addr>().is_ok()
            && (suffix.is_empty() || suffix.strip_prefix(':').is_some_and(valid_port));
    }

    if authority.matches(':').count() > 1 {
        return false;
    }

    let (host, port) = authority
        .rsplit_once(':')
        .map_or((authority, None), |(host, port)| (host, Some(port)));

    valid_host(host) && port.is_none_or(valid_port)
}

fn valid_port(port: &str) -> bool {
    !port.is_empty()
        && port.chars().all(|character| character.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|port| port != 0)
}

fn valid_host(host: &str) -> bool {
    let host = host.strip_suffix('.').unwrap_or(host);
    if host.is_empty() || host.chars().count() > 253 {
        return false;
    }

    if host.parse::<IpAddr>().is_ok() || host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    if host
        .chars()
        .all(|character| character.is_ascii_digit() || character == '.')
    {
        return false;
    }

    host.split('.').all(valid_host_label)
}

fn valid_host_label(label: &str) -> bool {
    let mut characters = label.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let last = label.chars().next_back().expect("label is not empty");

    label.chars().count() <= 63
        && first.is_alphanumeric()
        && last.is_alphanumeric()
        && label
            .chars()
            .all(|character| character.is_alphanumeric() || character == '-')
}

fn preview(url: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 100;

    let mut characters = url.chars();
    let mut preview: String = characters.by_ref().take(MAX_PREVIEW_CHARS).collect();
    if characters.next().is_some() {
        preview.push('…');
    }
    preview
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::storage::Settings;

    #[test]
    fn recognizes_http_https_and_www_urls() {
        assert_eq!(
            direct_url("HTTPS://Example.com/path?q=rust#top"),
            Some("https://Example.com/path?q=rust#top".to_owned())
        );
        assert_eq!(
            direct_url("http://localhost:8080"),
            Some("http://localhost:8080".to_owned())
        );
        assert_eq!(
            direct_url("www.rust-lang.org/learn"),
            Some("https://www.rust-lang.org/learn".to_owned())
        );
        assert_eq!(
            direct_url("https://[::1]:3000/"),
            Some("https://[::1]:3000/".to_owned())
        );
    }

    #[test]
    fn rejects_non_urls_and_unsafe_authorities() {
        for input in [
            "example.com",
            "www.",
            "https://",
            "https://exa mple.com",
            "https://user:pass@example.com",
            "https://example.com:0",
            "https://example.com:70000",
            "https://-example.com",
            "https://999.999.999.999",
            "file:///C:/Windows",
        ] {
            assert_eq!(direct_url(input), None, "unexpected URL match: {input}");
        }
    }

    #[test]
    fn provider_returns_open_and_copy_actions() {
        let provider = UrlProvider;
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
        let response = provider.query(&Query::new("www.example.com"), &context);

        assert!(response.exclusive);
        assert_eq!(response.results.len(), 1);
        assert!(matches!(
            response.results[0].actions.as_slice(),
            [
                Action::OpenUrl { url },
                Action::CopyText { text }
            ] if url == "https://www.example.com" && text == url
        ));
    }
}
