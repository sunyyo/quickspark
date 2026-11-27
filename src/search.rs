use pinyin::ToPinyin;

use crate::model::Usage;

pub(crate) fn search_keys(name: &str) -> (String, String) {
    let mut full = String::new();
    let mut initials = String::new();
    let mut previous = None;
    for ch in name.chars() {
        if let Some(py) = ch.to_pinyin() {
            // The compact dictionary is character-based; correct a frequent app-name phrase.
            let plain = if ch == '乐' && previous == Some('音') {
                "yue"
            } else {
                py.plain()
            };
            full.push_str(plain);
            if let Some(first) = plain.chars().next() {
                initials.push(first);
            }
        } else {
            for lower in ch.to_lowercase() {
                full.push(lower);
                if !lower.is_whitespace() {
                    initials.push(lower);
                }
            }
        }
        previous = Some(ch);
    }
    (full, initials)
}

pub(crate) fn fuzzy_score(text: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    if text == query {
        return Some(100_000);
    }
    if text.starts_with(query) {
        return Some(90_000 - text.len() as i32);
    }
    if let Some(pos) = text
        .char_indices()
        .filter(|(i, _)| {
            *i > 0
                && (text.as_bytes()[i - 1].is_ascii_whitespace()
                    || matches!(text.as_bytes()[i - 1], b'-' | b'_' | b'.'))
        })
        .find_map(|(i, _)| text[i..].starts_with(query).then_some(i))
    {
        return Some(82_000 - pos as i32 * 20 - text.len() as i32);
    }
    if let Some(pos) = text.find(query) {
        return Some(75_000 - pos as i32 * 20 - text.len() as i32);
    }
    let query_len = query.chars().count();
    if query_len < 3 {
        return None;
    }
    let mut text_chars = text.chars().enumerate();
    let mut first = None;
    let mut last = 0usize;
    for q in query.chars() {
        let (i, _) = text_chars.find(|(_, character)| *character == q)?;
        first.get_or_insert(i);
        last = i;
    }
    let start = first.unwrap_or(0);
    let span = last - start + 1;
    if span > query_len * 3 {
        return None;
    }
    Some(45_000 - start as i32 * 300 - (span - query_len) as i32 * 900 - text.len() as i32 * 3)
}

pub(crate) fn rank_entry(
    name: &str,
    pinyin: &str,
    initials: &str,
    query: &str,
    usage: Usage,
    now: u64,
) -> Option<i32> {
    let name = fuzzy_score(name, query);
    let pinyin = fuzzy_score(pinyin, query).map(|value| value - 3_000);
    let initials = fuzzy_score(initials, query).map(|value| value - 5_000);
    let base = [name, pinyin, initials].into_iter().flatten().max()?;
    let recent = if usage.last > 0 {
        let days = now.saturating_sub(usage.last) / 86_400;
        800i32.saturating_sub(days.min(80) as i32 * 10)
    } else {
        0
    };
    let boost = (usage.count.min(20) as i32 * 70 + recent).min(2_200);
    Some(base + boost)
}
