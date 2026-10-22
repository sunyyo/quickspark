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
    assert_ne!(light.bg, dark.bg);
    assert_ne!(light.answer_bg, dark.answer_bg);
    assert_ne!(light.text, dark.text);
    assert_ne!(light.muted, dark.muted);
    assert_ne!(light.selected, dark.selected);
    assert_ne!(light.divider, dark.divider);
    assert_ne!(light.code, dark.code);
    assert_ne!(light.link, dark.link);
}

#[test]
fn native_answer_controls_follow_reasoning_and_completion() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus, VK_RETURN, VK_TAB};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsWindowVisible, SendMessageW, WM_COMMAND, WM_KEYDOWN,
    };

    for light_theme in [true, false] {
        let window = super::show_ai_stream(std::ptr::null_mut(), "测试问题", 0, 0, 1, light_theme)
            .expect("answer window");
        let _cleanup = AnswerWindowGuard(window);
        let state = unsafe { &*super::answer_window_ptr(window) };
        assert_ne!(unsafe { IsWindowVisible(state.stop_button) }, 0);
        assert_eq!(unsafe { IsWindowVisible(state.thinking_copy) }, 0);

        assert!(super::append_ai_reasoning(window, "保留完整思考过程。"));
        unsafe {
            SetFocus(state.answer_edit);
            SendMessageW(state.answer_edit, WM_KEYDOWN, VK_TAB as usize, 0);
        }
        assert_eq!(unsafe { GetFocus() }, state.thinking_toggle);
        unsafe { SendMessageW(state.thinking_toggle, WM_KEYDOWN, VK_RETURN as usize, 0) };
        let state = unsafe { &*super::answer_window_ptr(window) };
        assert!(state.thinking_expanded);
        assert_ne!(unsafe { IsWindowVisible(state.thinking_copy) }, 0);
        assert_ne!(unsafe { IsWindowVisible(state.thinking_edit) }, 0);

        assert!(super::append_ai_answer(window, "这是一段回答。"));
        super::finish_ai_answer(window, Some("length"));
        let state = unsafe { &*super::answer_window_ptr(window) };
        assert!(!state.streaming);
        assert_eq!(unsafe { IsWindowVisible(state.stop_button) }, 0);
        assert_ne!(unsafe { IsWindowVisible(state.copy_button) }, 0);
        assert_ne!(unsafe { IsWindowVisible(state.close_button) }, 0);
        assert_eq!(control_text(state.hint), "输出达到长度上限，回答可能不完整");
        assert!(control_text(state.thinking_toggle).contains("收起思考过程"));

        unsafe { SendMessageW(window, WM_COMMAND, super::THINK_TOGGLE_ID, 0) };
        let state = unsafe { &*super::answer_window_ptr(window) };
        assert_eq!(unsafe { IsWindowVisible(state.thinking_copy) }, 0);
        assert_eq!(unsafe { IsWindowVisible(state.thinking_edit) }, 0);
        assert_eq!(control_text(state.thinking_toggle), "展开思考过程");
        assert_eq!(state.answer, "这是一段回答。");
        assert_eq!(state.reasoning, "保留完整思考过程。");
    }
}

#[test]
fn native_answer_controls_hide_stop_after_failure_or_cancellation() {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;

    for failed in [true, false] {
        let window = super::show_ai_stream(std::ptr::null_mut(), "测试问题", 0, 0, 1, true)
            .expect("answer window");
        let _cleanup = AnswerWindowGuard(window);
        if failed {
            super::fail_ai_answer(window, "测试连接失败");
        } else {
            let state = unsafe { &*super::answer_window_ptr(window) };
            unsafe {
                super::SetFocus(state.stop_button);
                super::notify_cancel(window);
            }
            assert_eq!(unsafe { super::GetFocus() }, state.answer_edit);
            super::cancel_ai_answer(window);
        }
        let state = unsafe { &*super::answer_window_ptr(window) };
        assert!(!state.streaming);
        assert_eq!(unsafe { IsWindowVisible(state.stop_button) }, 0);
        assert_eq!(unsafe { IsWindowVisible(state.thinking_toggle) }, 0);
        assert_eq!(
            control_text(state.hint),
            if failed {
                "请求失败"
            } else {
                "已停止生成"
            }
        );
        if failed {
            assert!(state.answer.contains("测试连接失败"));
        }
    }
}

struct AnswerWindowGuard(windows_sys::Win32::Foundation::HWND);

impl Drop for AnswerWindowGuard {
    fn drop(&mut self) {
        super::close_ai_answer(self.0);
    }
}

fn control_text(window: windows_sys::Win32::Foundation::HWND) -> String {
    let length = unsafe { super::GetWindowTextLengthW(window) };
    let mut text = vec![0u16; length as usize + 1];
    unsafe { super::GetWindowTextW(window, text.as_mut_ptr(), text.len() as i32) };
    String::from_utf16_lossy(&text[..length as usize])
}
