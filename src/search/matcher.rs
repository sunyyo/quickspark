use pinyin::ToPinyin;

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
            *i == 0
                || text.as_bytes()[i - 1].is_ascii_whitespace()
                || matches!(text.as_bytes()[i - 1], b'-' | b'_' | b'.')
        })
        .find_map(|(i, _)| text[i..].starts_with(query).then_some(i))
    {
        return Some(82_000 - pos as i32 * 20 - text.len() as i32);
    }
    if let Some(pos) = text.find(query) {
        return Some(75_000 - pos as i32 * 20 - text.len() as i32);
    }
    if query.chars().count() < 3 {
        return None;
    }
    let mut at = 0usize;
    let chars: Vec<char> = text.chars().collect();
    let mut first = None;
    let mut last = 0usize;
    for q in query.chars() {
        let mut found = None;
        for (i, character) in chars.iter().enumerate().skip(at) {
            if *character == q {
                found = Some(i);
                break;
            }
        }
        let i = found?;
        first.get_or_insert(i);
        last = i;
        at = i + 1;
    }
    let query_len = query.chars().count();
    let start = first.unwrap_or(0);
    let span = last - start + 1;
    if span > query_len * 3 {
        return None;
    }
    Some(45_000 - start as i32 * 300 - (span - query_len) as i32 * 900 - text.len() as i32 * 3)
}
