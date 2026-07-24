#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MarkdownStyle {
    Heading(u8),
    Bold,
    Italic,
    Strike,
    Code,
    Quote,
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MarkdownSpan {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) style: MarkdownStyle,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct RenderedMarkdown {
    pub(super) text: String,
    pub(super) spans: Vec<MarkdownSpan>,
}

#[derive(Default)]
struct Builder {
    rendered: RenderedMarkdown,
    utf16_len: usize,
}

impl Builder {
    fn position(&self) -> usize {
        self.utf16_len
    }

    fn push(&mut self, text: &str) {
        self.rendered.text.push_str(text);
        self.utf16_len += text.encode_utf16().count();
    }

    fn line_break(&mut self) {
        self.push("\r\n");
    }

    fn span(&mut self, start: usize, style: MarkdownStyle) {
        let end = self.position();
        if start < end {
            self.rendered.spans.push(MarkdownSpan { start, end, style });
        }
    }
}

pub(super) fn render_markdown(source: &str) -> RenderedMarkdown {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let mut builder = Builder::default();
    let mut fenced_code = false;

    for part in normalized.split_inclusive('\n') {
        let has_newline = part.ends_with('\n');
        let line = part.strip_suffix('\n').unwrap_or(part);
        let trimmed = line.trim_start();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced_code = !fenced_code;
            continue;
        }

        if fenced_code {
            let start = builder.position();
            builder.push(line);
            builder.span(start, MarkdownStyle::Code);
            if has_newline {
                builder.line_break();
            }
            continue;
        }

        render_line(line, &mut builder);
        if has_newline {
            builder.line_break();
        }
    }

    builder.rendered
}

fn render_line(line: &str, builder: &mut Builder) {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return;
    }

    if is_horizontal_rule(trimmed) {
        let start = builder.position();
        builder.push("────────────────────────");
        builder.span(start, MarkdownStyle::Quote);
        return;
    }

    if let Some((level, content)) = heading(trimmed) {
        let start = builder.position();
        render_inline(content, builder);
        builder.span(start, MarkdownStyle::Heading(level));
        return;
    }

    if let Some(content) = trimmed
        .strip_prefix("> ")
        .or_else(|| trimmed.strip_prefix('>'))
    {
        let start = builder.position();
        builder.push("│ ");
        render_inline(content.trim_start(), builder);
        builder.span(start, MarkdownStyle::Quote);
        return;
    }

    if let Some(content) =
        checkbox_item(trimmed, "- [ ] ").or_else(|| checkbox_item(trimmed, "* [ ] "))
    {
        builder.push("☐  ");
        render_inline(content, builder);
        return;
    }
    if let Some(content) = checkbox_item(trimmed, "- [x] ")
        .or_else(|| checkbox_item(trimmed, "- [X] "))
        .or_else(|| checkbox_item(trimmed, "* [x] "))
        .or_else(|| checkbox_item(trimmed, "* [X] "))
    {
        builder.push("☑  ");
        render_inline(content, builder);
        return;
    }

    if let Some(content) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        builder.push("•  ");
        render_inline(content, builder);
        return;
    }

    if is_table_separator(trimmed) {
        let start = builder.position();
        builder.push("────────┼────────");
        builder.span(start, MarkdownStyle::Quote);
        return;
    }
    if trimmed.starts_with('|') && trimmed.ends_with('|') {
        render_table_row(trimmed, builder);
        return;
    }

    render_inline(line, builder);
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let count = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&count) || line.as_bytes().get(count) != Some(&b' ') {
        return None;
    }
    Some((
        count as u8,
        line[count + 1..].trim_end_matches('#').trim_end(),
    ))
}

fn checkbox_item<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.strip_prefix(prefix)
}

fn is_horizontal_rule(line: &str) -> bool {
    let compact = line.chars().filter(|character| *character != ' ');
    let chars = compact.collect::<Vec<_>>();
    chars.len() >= 3
        && matches!(chars.first(), Some('-' | '*' | '_'))
        && chars
            .iter()
            .all(|character| Some(character) == chars.first())
}

fn is_table_separator(line: &str) -> bool {
    if !line.contains('|') {
        return false;
    }
    line.trim_matches('|').split('|').all(|cell| {
        let cell = cell.trim().trim_matches(':');
        cell.len() >= 3 && cell.bytes().all(|byte| byte == b'-')
    })
}

fn render_table_row(line: &str, builder: &mut Builder) {
    for (index, cell) in line.trim_matches('|').split('|').enumerate() {
        if index > 0 {
            builder.push("  │  ");
        }
        render_inline(cell.trim(), builder);
    }
}

fn render_inline(input: &str, builder: &mut Builder) {
    let mut rest = input;
    while !rest.is_empty() {
        if let Some(escaped) = rest.strip_prefix('\\')
            && let Some(character) = escaped.chars().next()
        {
            builder.push(&character.to_string());
            rest = &escaped[character.len_utf8()..];
            continue;
        }

        if let Some(after) = rest.strip_prefix("![")
            && let Some((label, url, remaining)) = markdown_link(after)
        {
            let start = builder.position();
            builder.push("图片：");
            render_inline(label, builder);
            if !url.is_empty() {
                builder.push(" (");
                builder.push(url);
                builder.push(")");
            }
            builder.span(start, MarkdownStyle::Link);
            rest = remaining;
            continue;
        }

        if let Some(after) = rest.strip_prefix('[')
            && let Some((label, url, remaining)) = markdown_link(after)
        {
            let start = builder.position();
            render_inline(label, builder);
            if !url.is_empty() {
                builder.push(" (");
                builder.push(url);
                builder.push(")");
            }
            builder.span(start, MarkdownStyle::Link);
            rest = remaining;
            continue;
        }

        if let Some((inner, remaining)) = delimited(rest, "**").or_else(|| delimited(rest, "__")) {
            let start = builder.position();
            render_inline(inner, builder);
            builder.span(start, MarkdownStyle::Bold);
            rest = remaining;
            continue;
        }

        if let Some((inner, remaining)) = delimited(rest, "~~") {
            let start = builder.position();
            render_inline(inner, builder);
            builder.span(start, MarkdownStyle::Strike);
            rest = remaining;
            continue;
        }

        if let Some((inner, remaining)) = delimited(rest, "`") {
            let start = builder.position();
            builder.push(inner);
            builder.span(start, MarkdownStyle::Code);
            rest = remaining;
            continue;
        }

        if let Some((inner, remaining)) = delimited(rest, "*").or_else(|| delimited(rest, "_")) {
            let start = builder.position();
            render_inline(inner, builder);
            builder.span(start, MarkdownStyle::Italic);
            rest = remaining;
            continue;
        }

        if let Some(remaining) = rest
            .strip_prefix("<br>")
            .or_else(|| rest.strip_prefix("<br/>"))
        {
            builder.line_break();
            rest = remaining;
            continue;
        }

        let character = rest.chars().next().expect("rest is not empty");
        builder.push(&character.to_string());
        rest = &rest[character.len_utf8()..];
    }
}

fn delimited<'a>(input: &'a str, delimiter: &str) -> Option<(&'a str, &'a str)> {
    let after = input.strip_prefix(delimiter)?;
    let closing = after.find(delimiter)?;
    if closing == 0 {
        return None;
    }
    Some((&after[..closing], &after[closing + delimiter.len()..]))
}

fn markdown_link(input: &str) -> Option<(&str, &str, &str)> {
    let label_end = input.find("](")?;
    let after_label = &input[label_end + 2..];
    let url_end = after_label.find(')')?;
    Some((
        &input[..label_end],
        &after_label[..url_end],
        &after_label[url_end + 1..],
    ))
}

#[cfg(test)]
mod tests {
    use super::{MarkdownStyle, render_markdown};

    #[test]
    fn removes_common_markdown_markers_and_records_styles() {
        let rendered =
            render_markdown("# 标题\n**粗体**、*斜体*、`code` 和 ~~删除~~\n- 项目\n> 引用");
        assert_eq!(
            rendered.text,
            "标题\r\n粗体、斜体、code 和 删除\r\n•  项目\r\n│ 引用"
        );
        assert!(
            rendered
                .spans
                .iter()
                .any(|span| span.style == MarkdownStyle::Heading(1))
        );
        for style in [
            MarkdownStyle::Bold,
            MarkdownStyle::Italic,
            MarkdownStyle::Code,
            MarkdownStyle::Strike,
            MarkdownStyle::Quote,
        ] {
            assert!(rendered.spans.iter().any(|span| span.style == style));
        }
    }

    #[test]
    fn renders_code_fences_links_checkboxes_and_tables() {
        let rendered = render_markdown(
            "```rust\nfn main() {}\n```\n- [x] 完成\n[官网](https://example.com)\n| A | B |\n|---|---|",
        );
        assert_eq!(
            rendered.text,
            "fn main() {}\r\n☑  完成\r\n官网 (https://example.com)\r\nA  │  B\r\n────────┼────────"
        );
        assert!(
            rendered
                .spans
                .iter()
                .any(|span| span.style == MarkdownStyle::Code)
        );
        assert!(
            rendered
                .spans
                .iter()
                .any(|span| span.style == MarkdownStyle::Link)
        );
    }

    #[test]
    fn span_offsets_count_utf16_code_units() {
        let rendered = render_markdown("😀 **好**");
        let bold = rendered
            .spans
            .iter()
            .find(|span| span.style == MarkdownStyle::Bold)
            .unwrap();
        assert_eq!((bold.start, bold.end), (3, 4));
    }
}
