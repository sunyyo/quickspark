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
