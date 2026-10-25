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
        let (answer_edit, toggle) = inspect(window, |state| {
            assert_ne!(unsafe { IsWindowVisible(state.stop_button) }, 0);
            assert_eq!(unsafe { IsWindowVisible(state.thinking_copy) }, 0);
            (state.answer_edit, state.thinking_toggle)
        });
        assert!(super::append_ai_reasoning(window, "保留完整思考过程。"));
        unsafe {
            SetFocus(answer_edit);
            SendMessageW(answer_edit, WM_KEYDOWN, VK_TAB as usize, 0);
        }
        assert_eq!(unsafe { GetFocus() }, toggle);
        unsafe { SendMessageW(toggle, WM_KEYDOWN, VK_RETURN as usize, 0) };
        inspect(window, |state| {
            assert!(state.thinking_expanded);
            assert_ne!(unsafe { IsWindowVisible(state.thinking_copy) }, 0);
            assert_ne!(unsafe { IsWindowVisible(state.thinking_edit) }, 0);
        });
        assert!(super::append_ai_answer(window, "这是一段回答。"));
        super::finish_ai_answer(window, Some("length"));
        inspect(window, |state| {
            assert!(!state.streaming);
            assert_eq!(unsafe { IsWindowVisible(state.stop_button) }, 0);
            assert_ne!(unsafe { IsWindowVisible(state.copy_button) }, 0);
            assert_ne!(unsafe { IsWindowVisible(state.close_button) }, 0);
            assert_eq!(control_text(state.hint), "输出达到长度上限，回答可能不完整");
            assert!(control_text(state.thinking_toggle).contains("收起思考过程"));
        });
        unsafe { SendMessageW(window, WM_COMMAND, super::THINK_TOGGLE_ID, 0) };
        inspect(window, |state| {
            assert_eq!(unsafe { IsWindowVisible(state.thinking_copy) }, 0);
            assert_eq!(unsafe { IsWindowVisible(state.thinking_edit) }, 0);
            assert_eq!(control_text(state.thinking_toggle), "展开思考过程");
            assert_eq!(state.answer, "这是一段回答。");
            assert_eq!(state.reasoning, "保留完整思考过程。");
        });
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
            let (stop, answer) = inspect(window, |state| (state.stop_button, state.answer_edit));
            unsafe {
                super::SetFocus(stop);
                super::notify_cancel(window);
            }
            assert_eq!(unsafe { super::GetFocus() }, answer);
            super::cancel_ai_answer(window);
        }
        inspect(window, |state| {
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
        });
    }
}

#[test]
fn native_answer_paint_can_reenter_during_a_model_borrow() {
    use windows_sys::Win32::{
        Graphics::Gdi::{GetDC, ReleaseDC},
        UI::{
            Controls::{DRAWITEMSTRUCT, ODA_DRAWENTIRE, ODT_BUTTON},
            WindowsAndMessaging::{SendMessageW, WM_CTLCOLORSTATIC, WM_DRAWITEM},
        },
    };
    let window = super::show_ai_stream(std::ptr::null_mut(), "重入测试", 0, 0, 1, true).unwrap();
    let _cleanup = AnswerWindowGuard(window);
    let dc = unsafe { GetDC(window) };
    assert!(!dc.is_null());
    super::with_answer_state(window, |state| {
        assert!(super::with_answer_state(window, |_| ()).is_none());
        assert_eq!(
            unsafe { SendMessageW(window, WM_CTLCOLORSTATIC, dc as usize, state.hint as isize) },
            state.window_brush as isize
        );
        let draw = DRAWITEMSTRUCT {
            CtlType: ODT_BUTTON,
            CtlID: super::COPY_BUTTON_ID as u32,
            itemID: 0,
            itemAction: ODA_DRAWENTIRE,
            itemState: 0,
            hwndItem: state.copy_button,
            hDC: dc,
            rcItem: super::RECT {
                left: 0,
                top: 0,
                right: 108,
                bottom: 32,
            },
            itemData: 0,
        };
        assert_eq!(
            unsafe {
                SendMessageW(
                    window,
                    WM_DRAWITEM,
                    super::COPY_BUTTON_ID,
                    &draw as *const _ as isize,
                )
            },
            1
        );
        unsafe { super::layout(window, state) };
    })
    .unwrap();
    unsafe { ReleaseDC(window, dc) };
    let handle = window as usize;
    assert!(
        std::thread::spawn(move || super::with_answer_state(handle as _, |_| ()).is_none())
            .join()
            .unwrap()
    );
    assert!(super::append_ai_answer(window, "同步绘制后仍可更新"));
    inspect(window, |state| {
        assert_eq!(state.answer, "同步绘制后仍可更新")
    });
}

#[test]
fn native_answer_destruction_releases_resources_after_the_last_callback() {
    use windows_sys::Win32::{
        Graphics::Gdi::{GetObjectW, HGDIOBJ},
        UI::WindowsAndMessaging::{
            CreateWindowExW, HWND_MESSAGE, IsWindow, MSG, PM_REMOVE, PeekMessageW, SendMessageW,
            WM_APP, WM_CLOSE,
        },
    };
    for reentrant in [false, true] {
        RESOURCE_RELEASES.with(|releases| releases.borrow_mut().clear());
        let owner = unsafe {
            CreateWindowExW(
                0,
                super::wide("STATIC").as_ptr(),
                std::ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                super::GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            )
        };
        assert!(!owner.is_null());
        let _owner_cleanup = AnswerWindowGuard(owner);
        let cancel = WM_APP + 101;
        let closed = cancel + 1;
        let window = super::show_ai_stream(owner, "销毁测试", cancel, closed, 42, true).unwrap();
        let _cleanup = AnswerWindowGuard(window);
        let shared = unsafe { super::answer_window_state(window) }.unwrap();
        let weak = std::rc::Rc::downgrade(&shared);
        drop(shared);
        let resources: [HGDIOBJ; 6] = inspect(window, |state| {
            [
                state.heading_font as _,
                state.body_font as _,
                state.answer_font as _,
                state.hint_font as _,
                state.window_brush as _,
                state.answer_brush as _,
            ]
        });
        if reentrant {
            super::with_answer_state(window, |state| {
                unsafe { SendMessageW(window, WM_CLOSE, 0, 0) };
                assert_eq!(unsafe { IsWindow(window) }, 0);
                assert!(unsafe { super::answer_window_state(window) }.is_none());
                assert!(weak.upgrade().is_some());
                RESOURCE_RELEASES.with(|releases| assert!(releases.borrow().is_empty()));
                for resource in resources {
                    assert_ne!(unsafe { GetObjectW(resource, 0, std::ptr::null_mut()) }, 0);
                }
                state.answer.push_str("回调结束前状态仍有效");
            })
            .unwrap();
        } else {
            unsafe { SendMessageW(window, WM_CLOSE, 0, 0) };
        }
        assert!(weak.upgrade().is_none());
        // On this host GetObject can still recognize a logical font after a
        // successful release. Verify the ownership contract directly: exactly
        // one successful DeleteObject call per owned resource, after callbacks.
        let released =
            RESOURCE_RELEASES.with(|releases| std::mem::take(&mut *releases.borrow_mut()));
        assert_eq!(released.len(), resources.len());
        let mut expected = resources.map(|handle| handle as usize).to_vec();
        let mut actual = released
            .iter()
            .map(|(handle, _)| *handle)
            .collect::<Vec<_>>();
        expected.sort_unstable();
        actual.sort_unstable();
        assert_eq!(actual, expected);
        assert!(
            released.iter().all(|(_, success)| *success != 0),
            "resource releases: {released:?}"
        );
        assert!(!super::append_ai_answer(window, "迟到的结果"));
        let mut notifications = Vec::new();
        let mut message: MSG = unsafe { std::mem::zeroed() };
        while unsafe { PeekMessageW(&mut message, owner, cancel, closed, PM_REMOVE) } != 0 {
            assert_eq!(message.wParam, 42);
            assert_eq!(message.lParam, window as isize);
            notifications.push(message.message);
        }
        assert_eq!(notifications, vec![cancel, closed]);
    }
}

thread_local! {
    static RESOURCE_RELEASES: std::cell::RefCell<Vec<(usize, i32)>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(super) fn record_resource_release(handle: usize, success: i32) {
    RESOURCE_RELEASES.with(|releases| releases.borrow_mut().push((handle, success)));
}

fn inspect<R>(
    window: windows_sys::Win32::Foundation::HWND,
    access: impl FnOnce(&super::AnswerWindow) -> R,
) -> R {
    super::with_answer_state(window, |state| access(state)).expect("live window state")
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
