use super::{
    CharFormatW,
    rendering::{answer_display, reasoning_tail, rich_edit_available},
};

#[test]
fn native_rich_edit_is_available_and_char_format_matches_windows_abi() {
    assert!(rich_edit_available());
    assert_eq!(std::mem::size_of::<CharFormatW>(), 92);
}

#[test]
fn long_reasoning_preview_keeps_unicode_tail() {
    assert_eq!(reasoning_tail("一二三四五", 3), ("三四五", true));
    assert_eq!(reasoning_tail("😀结果", 10), ("😀结果", false));
}

#[test]
fn long_answer_display_keeps_full_copy_source_but_bounds_rendering() {
    let answer = "答".repeat(super::ANSWER_DISPLAY_CHARS + 2);
    let display = answer_display(&answer);
    assert!(display.starts_with("[回答较长"));
    assert!(display.ends_with(&"答".repeat(super::ANSWER_DISPLAY_CHARS)));
    assert_eq!(answer.chars().count(), super::ANSWER_DISPLAY_CHARS + 2);
}

#[test]
fn answer_palette_changes_every_visible_color_in_dark_mode() {
    let light = super::answer_colors(true);
    let dark = super::answer_colors(false);
    assert_ne!(light.window_bg, dark.window_bg);
    assert_ne!(light.answer_bg, dark.answer_bg);
    assert_ne!(light.text, dark.text);
    assert_ne!(light.muted, dark.muted);
    assert_ne!(light.code, dark.code);
    assert_ne!(light.link, dark.link);
}
