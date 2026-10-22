use super::*;

pub(super) fn rich_edit_available() -> bool {
    *RICH_EDIT_LOADED
        .get_or_init(|| unsafe { !LoadLibraryW(wide("Msftedit.dll").as_ptr()).is_null() })
}

pub(super) unsafe fn render_answer(state: &AnswerWindow) {
    let display = answer_display(&state.answer);
    let rendered = render_markdown(&display);
    let end = rendered.text.encode_utf16().count();
    unsafe {
        SendMessageW(state.answer_edit, WM_SETREDRAW, 0, 0);
        SetWindowTextW(state.answer_edit, wide(&rendered.text).as_ptr());
        if state.rich_edit {
            SendMessageW(
                state.answer_edit,
                EM_SETBKGNDCOLOR,
                0,
                state.colors.answer_bg as LPARAM,
            );
            apply_base_format(state.answer_edit, end, state.colors.text);
        }
    }
    if state.rich_edit {
        for span in &rendered.spans {
            unsafe { apply_markdown_span(state.answer_edit, span, state.colors) };
        }
    }
    unsafe {
        SendMessageW(state.answer_edit, EM_SETSEL, end, end as LPARAM);
        SendMessageW(state.answer_edit, EM_SCROLLCARET, 0, 0);
        SendMessageW(state.answer_edit, WM_SETREDRAW, 1, 0);
        InvalidateRect(state.answer_edit, null(), 0);
        UpdateWindow(state.answer_edit);
    }
}

pub(super) unsafe fn render_plain_answer(state: &AnswerWindow) {
    let display = answer_display(&state.answer);
    let text = normalize_newlines(&display);
    let end = text.encode_utf16().count();
    unsafe {
        SendMessageW(state.answer_edit, WM_SETREDRAW, 0, 0);
        SetWindowTextW(state.answer_edit, wide(&text).as_ptr());
        if state.rich_edit {
            SendMessageW(
                state.answer_edit,
                EM_SETBKGNDCOLOR,
                0,
                state.colors.answer_bg as LPARAM,
            );
            apply_base_format(state.answer_edit, end, state.colors.text);
        }
        SendMessageW(state.answer_edit, EM_SETSEL, end, end as LPARAM);
        SendMessageW(state.answer_edit, EM_SCROLLCARET, 0, 0);
        SendMessageW(state.answer_edit, WM_SETREDRAW, 1, 0);
        InvalidateRect(state.answer_edit, null(), 0);
    }
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
    let end = text.encode_utf16().count();
    unsafe {
        SendMessageW(state.thinking_edit, WM_SETREDRAW, 0, 0);
        SetWindowTextW(state.thinking_edit, wide(&text).as_ptr());
        if state.thinking_rich_edit {
            SendMessageW(
                state.thinking_edit,
                EM_SETBKGNDCOLOR,
                0,
                state.colors.answer_bg as LPARAM,
            );
            apply_text_format(
                state.thinking_edit,
                end,
                240,
                state.colors.dim,
                "Microsoft YaHei UI",
            );
        }
        SendMessageW(state.thinking_edit, EM_SETSEL, end, end as LPARAM);
        SendMessageW(state.thinking_edit, EM_SCROLLCARET, 0, 0);
        SendMessageW(state.thinking_edit, WM_SETREDRAW, 1, 0);
        InvalidateRect(state.thinking_edit, null(), 0);
        UpdateWindow(state.thinking_edit);
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

unsafe fn apply_base_format(edit: HWND, end: usize, text_color: u32) {
    unsafe {
        apply_text_format(
            edit,
            end,
            ANSWER_FONT_TWIPS,
            text_color,
            "Microsoft YaHei UI",
        )
    };
}

unsafe fn apply_text_format(edit: HWND, end: usize, height: i32, color: u32, face: &str) {
    let mut format = char_format(
        CFM_BOLD | CFM_ITALIC | CFM_UNDERLINE | CFM_STRIKEOUT | CFM_COLOR | CFM_SIZE | CFM_FACE,
        0,
    );
    format.height = height;
    format.text_color = color;
    set_face(&mut format, face);
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
    let mut format = match span.style {
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
            set_face(&mut format, "Consolas");
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
    format.cb_size = std::mem::size_of::<CharFormatW>() as u32;
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

fn set_face(format: &mut CharFormatW, face: &str) {
    for (destination, source) in format
        .face_name
        .iter_mut()
        .zip(face.encode_utf16().chain(Some(0)))
    {
        *destination = source;
    }
}
