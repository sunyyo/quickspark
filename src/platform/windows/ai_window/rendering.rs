use super::*;

pub(super) fn rich_edit_available() -> bool {
    *RICH_EDIT_LOADED
        .get_or_init(|| unsafe { !LoadLibraryW(wide("Msftedit.dll").as_ptr()).is_null() })
}

pub(super) unsafe fn render_answer(state: &AnswerWindow, scroll_to_end: bool) {
    let display = answer_display(&state.answer);
    let rendered = render_markdown(&display);
    unsafe {
        render_edit_text(EditRender {
            edit: state.answer_edit,
            rich_edit: state.rich_edit,
            text: &rendered.text,
            spans: &rendered.spans,
            colors: state.colors,
            text_height: ANSWER_FONT_TWIPS,
            text_color: state.colors.text,
            scroll_to_end,
            update_now: true,
        })
    };
}

pub(super) unsafe fn render_plain_answer(state: &AnswerWindow) {
    let display = answer_display(&state.answer);
    let text = normalize_newlines(&display);
    unsafe {
        render_edit_text(EditRender {
            edit: state.answer_edit,
            rich_edit: state.rich_edit,
            text: &text,
            spans: &[],
            colors: state.colors,
            text_height: ANSWER_FONT_TWIPS,
            text_color: state.colors.text,
            scroll_to_end: true,
            update_now: false,
        })
    };
}

pub(super) fn answer_display(value: &str) -> Cow<'_, str> {
    let (tail, truncated) = reasoning_tail(value, ANSWER_DISPLAY_CHARS);
    if truncated {
        Cow::Owned(format!(
            "[回答较长，仅显示最近 {ANSWER_DISPLAY_CHARS} 字；“复制回答”仍会复制完整内容]\n\n{tail}"
        ))
    } else {
        Cow::Borrowed(value)
    }
}

pub(super) unsafe fn render_thinking(state: &AnswerWindow) {
    let (tail, truncated) = reasoning_tail(&state.reasoning, THINK_DISPLAY_CHARS);
    let text = if truncated {
        format!(
            "[思考过程较长，仅显示最近 {THINK_DISPLAY_CHARS} 字；“复制思考”可复制完整内容]\n\n{tail}"
        )
    } else {
        tail.to_owned()
    };
    let text = normalize_newlines(&text);
    unsafe {
        render_edit_text(EditRender {
            edit: state.thinking_edit,
            rich_edit: state.thinking_rich_edit,
            text: &text,
            spans: &[],
            colors: state.colors,
            text_height: 240,
            text_color: state.colors.dim,
            scroll_to_end: true,
            update_now: true,
        })
    };
}

struct EditRender<'a> {
    edit: HWND,
    rich_edit: bool,
    text: &'a str,
    spans: &'a [MarkdownSpan],
    colors: AnswerColors,
    text_height: i32,
    text_color: u32,
    scroll_to_end: bool,
    update_now: bool,
}

unsafe fn render_edit_text(render: EditRender<'_>) {
    let wide_text = wide(render.text);
    let end = wide_text.len() - 1;
    unsafe {
        SendMessageW(render.edit, WM_SETREDRAW, 0, 0);
        SetWindowTextW(render.edit, wide_text.as_ptr());
        if render.rich_edit {
            SendMessageW(
                render.edit,
                EM_SETBKGNDCOLOR,
                0,
                render.colors.answer_bg as LPARAM,
            );
            apply_text_format(
                render.edit,
                end,
                render.text_height,
                render.text_color,
                "Microsoft YaHei UI",
            );
            for span in render.spans {
                apply_markdown_span(render.edit, span, render.colors);
            }
        }
        let caret = if render.scroll_to_end { end } else { 0 };
        SendMessageW(render.edit, EM_SETSEL, caret, caret as LPARAM);
        SendMessageW(render.edit, EM_SCROLLCARET, 0, 0);
        SendMessageW(render.edit, WM_SETREDRAW, 1, 0);
        InvalidateRect(render.edit, null(), 0);
        if render.update_now {
            UpdateWindow(render.edit);
        }
    }
}

pub(super) fn reasoning_tail(value: &str, max_chars: usize) -> (&str, bool) {
    let mut reversed = value.char_indices().rev();
    let mut start = 0;
    for _ in 0..max_chars {
        let Some((index, _)) = reversed.next() else {
            return (value, false);
        };
        start = index;
    }
    if reversed.next().is_some() {
        (&value[start..], true)
    } else {
        (value, false)
    }
}

unsafe fn apply_text_format(edit: HWND, end: usize, height: i32, color: u32, face: &str) {
    let mut format = char_format(
        CFM_BOLD | CFM_ITALIC | CFM_UNDERLINE | CFM_STRIKEOUT | CFM_COLOR | CFM_SIZE | CFM_FACE,
        0,
    );
    format.height = height;
    format.text_color = color;
    copy_wide(face, &mut format.face_name);
    unsafe {
        SendMessageW(edit, EM_SETSEL, 0, end as LPARAM);
        SendMessageW(
            edit,
            EM_SETCHARFORMAT,
            SCF_SELECTION,
            &format as *const CharFormatW as LPARAM,
        );
    }
}

unsafe fn apply_markdown_span(edit: HWND, span: &MarkdownSpan, colors: AnswerColors) {
    let format = match span.style {
        MarkdownStyle::Heading(level) => {
            let mut format = char_format(CFM_BOLD | CFM_SIZE | CFM_COLOR, CFE_BOLD);
            format.height = match level {
                1 => 380,
                2 => 340,
                3 => 315,
                4 => 295,
                5 => 280,
                _ => 270,
            };
            format.text_color = colors.text;
            format
        }
        MarkdownStyle::Bold => char_format(CFM_BOLD, CFE_BOLD),
        MarkdownStyle::Italic => char_format(CFM_ITALIC, CFE_ITALIC),
        MarkdownStyle::Strike => char_format(CFM_STRIKEOUT, CFE_STRIKEOUT),
        MarkdownStyle::Code => {
            let mut format = char_format(CFM_FACE | CFM_COLOR, 0);
            format.text_color = colors.code;
            copy_wide("Consolas", &mut format.face_name);
            format
        }
        MarkdownStyle::Quote => {
            let mut format = char_format(CFM_ITALIC | CFM_COLOR, CFE_ITALIC);
            format.text_color = colors.muted;
            format
        }
        MarkdownStyle::Link => {
            let mut format = char_format(CFM_UNDERLINE | CFM_COLOR, CFE_UNDERLINE);
            format.text_color = colors.link;
            format
        }
    };
    unsafe {
        SendMessageW(edit, EM_SETSEL, span.start, span.end as LPARAM);
        SendMessageW(
            edit,
            EM_SETCHARFORMAT,
            SCF_SELECTION,
            &format as *const CharFormatW as LPARAM,
        );
    }
}

fn char_format(mask: u32, effects: u32) -> CharFormatW {
    CharFormatW {
        cb_size: std::mem::size_of::<CharFormatW>() as u32,
        mask,
        effects,
        height: 0,
        offset: 0,
        text_color: 0,
        charset: DEFAULT_CHARSET,
        pitch_and_family: FF_DONTCARE,
        face_name: [0; 32],
    }
}
