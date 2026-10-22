use std::{
    borrow::Cow,
    ptr::{null, null_mut},
    sync::OnceLock,
    time::Instant,
};

use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute},
    Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, CreateSolidBrush,
        DEFAULT_CHARSET, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE, DT_VCENTER,
        DeleteObject, DrawFocusRect, DrawTextW, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, FillRect,
        HBRUSH, HDC, HFONT, InvalidateRect, OUT_DEFAULT_PRECIS, PS_SOLID, RoundRect, SelectObject,
        SetBkColor, SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
    },
    System::LibraryLoader::{GetModuleHandleW, LoadLibraryW},
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED, SetWindowTheme},
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            EnableWindow, GetFocus, GetKeyState, SetFocus, VK_CONTROL, VK_ESCAPE, VK_RETURN,
            VK_SHIFT, VK_TAB,
        },
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            BM_CLICK, BS_OWNERDRAW, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW,
            DefWindowProcW, DestroyWindow, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY,
            GetClientRect, GetNextDlgTabItem, GetWindowLongPtrW, GetWindowRect,
            GetWindowTextLengthW, GetWindowTextW, IDC_ARROW, IsWindow, KillTimer, LoadCursorW,
            MB_ICONERROR, MB_OK, MINMAXINFO, MessageBoxW, MoveWindow, PostMessageW, RegisterClassW,
            SW_HIDE, SW_SHOW, SendMessageW, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
            SetWindowTextW, ShowWindow, WM_CLOSE, WM_COMMAND, WM_COPY, WM_CREATE, WM_CTLCOLORBTN,
            WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM, WM_ERASEBKGND,
            WM_GETMINMAXINFO, WM_KEYDOWN, WM_NCDESTROY, WM_SETFOCUS, WM_SETFONT, WM_SETREDRAW,
            WM_SIZE, WM_TIMER, WNDCLASSW, WS_CHILD, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
            WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
        },
    },
};

use super::clipboard::copy_text;
use super::markdown::{MarkdownSpan, MarkdownStyle, render_markdown};
use super::theme::{Theme as AnswerColors, theme as answer_colors};
use super::wide;

mod rendering;

use rendering::{render_answer, render_plain_answer, render_thinking, rich_edit_available};

const CLASS_NAME: &str = "QuickSparkAiAnswerWindow";
const DEFAULT_WINDOW_WIDTH: i32 = 860;
const DEFAULT_WINDOW_HEIGHT: i32 = 620;
const ANSWER_FONT_PIXELS: i32 = 19;
const ANSWER_FONT_TWIPS: i32 = ANSWER_FONT_PIXELS * 15;
const COPY_BUTTON_ID: usize = 4101;
const STOP_BUTTON_ID: usize = 4102;
const CLOSE_BUTTON_ID: usize = 4103;
const THINK_TOGGLE_ID: usize = 4104;
const COPY_THINKING_ID: usize = 4105;
const EDIT_SUBCLASS_ID: usize = 1;
const THINK_EDIT_SUBCLASS_ID: usize = 2;
const BUTTON_SUBCLASS_ID: usize = 3;
const RENDER_TIMER_ID: usize = 1;
const RENDER_INTERVAL_MS: u32 = 40;
const ANSWER_DISPLAY_CHARS: usize = 200_000;
const THINK_RENDER_TIMER_ID: usize = 2;
const THINK_RENDER_INTERVAL_MS: u32 = 100;
const THINK_DISPLAY_CHARS: usize = 10_000;
const EM_SETMARGINS: u32 = 0x00D3;
const EM_SETRECT: u32 = 0x00B3;
const EM_SETSEL: u32 = 0x00B1;
const EM_SCROLLCARET: u32 = 0x00B7;
const EM_SETBKGNDCOLOR: u32 = 0x0443;
const EM_SETCHARFORMAT: u32 = 0x0444;
const EC_LEFTMARGIN: usize = 0x0001;
const EC_RIGHTMARGIN: usize = 0x0002;
const SCF_SELECTION: usize = 0x0001;
const CFM_BOLD: u32 = 0x0000_0001;
const CFM_ITALIC: u32 = 0x0000_0002;
const CFM_UNDERLINE: u32 = 0x0000_0004;
const CFM_STRIKEOUT: u32 = 0x0000_0008;
const CFM_COLOR: u32 = 0x4000_0000;
const CFM_SIZE: u32 = 0x8000_0000;
const CFM_FACE: u32 = 0x2000_0000;
const CFE_BOLD: u32 = 0x0000_0001;
const CFE_ITALIC: u32 = 0x0000_0002;
const CFE_UNDERLINE: u32 = 0x0000_0004;
const CFE_STRIKEOUT: u32 = 0x0000_0008;
#[repr(C)]
struct CharFormatW {
    cb_size: u32,
    mask: u32,
    effects: u32,
    height: i32,
    offset: i32,
    text_color: u32,
    charset: u8,
    pitch_and_family: u8,
    face_name: [u16; 32],
}

struct AnswerInit {
    prompt: String,
    answer: String,
    reasoning: String,
    owner: HWND,
    cancel_message: u32,
    closed_message: u32,
    generation: u64,
    streaming: bool,
    light_theme: bool,
}

struct AnswerWindow {
    question_label: HWND,
    prompt: HWND,
    answer_label: HWND,
    answer_edit: HWND,
    thinking_toggle: HWND,
    thinking_copy: HWND,
    thinking_edit: HWND,
    hint: HWND,
    copy_button: HWND,
    stop_button: HWND,
    close_button: HWND,
    heading_font: HFONT,
    body_font: HFONT,
    answer_font: HFONT,
    hint_font: HFONT,
    window_brush: HBRUSH,
    answer_brush: HBRUSH,
    colors: AnswerColors,
    answer: String,
    reasoning: String,
    reasoning_chars: usize,
    rich_edit: bool,
    thinking_rich_edit: bool,
    render_dirty: bool,
    thinking_dirty: bool,
    thinking_visible: bool,
    thinking_expanded: bool,
    thinking_complete: bool,
    thinking_started: Option<Instant>,
    thinking_elapsed_ms: Option<u128>,
    owner: HWND,
    cancel_message: u32,
    closed_message: u32,
    generation: u64,
    streaming: bool,
}

static REGISTERED: OnceLock<bool> = OnceLock::new();
static RICH_EDIT_LOADED: OnceLock<bool> = OnceLock::new();

macro_rules! answer_state {
    ($window:expr) => {{
        #[allow(unused_unsafe)]
        unsafe {
            answer_window_ptr($window).as_mut()
        }
    }};
}

pub(crate) fn show_ai_answer(
    owner: HWND,
    prompt: &str,
    answer: &str,
    reasoning: &str,
    light_theme: bool,
) -> bool {
    show_answer_window(AnswerInit {
        prompt: normalize_newlines(prompt),
        answer: normalize_newlines(answer),
        reasoning: normalize_newlines(reasoning),
        owner,
        cancel_message: 0,
        closed_message: 0,
        generation: 0,
        streaming: false,
        light_theme,
    })
    .is_some()
}

pub(crate) fn show_ai_stream(
    owner: HWND,
    prompt: &str,
    cancel_message: u32,
    closed_message: u32,
    generation: u64,
    light_theme: bool,
) -> Option<HWND> {
    show_answer_window(AnswerInit {
        prompt: normalize_newlines(prompt),
        answer: String::new(),
        reasoning: String::new(),
        owner,
        cancel_message,
        closed_message,
        generation,
        streaming: true,
        light_theme,
    })
}

pub(crate) fn update_ai_status(window: HWND, status: &str) -> bool {
    if window.is_null() || unsafe { IsWindow(window) } == 0 || status.is_empty() {
        return false;
    }
    let Some(state) = answer_state!(window) else {
        return false;
    };
    if !state.streaming {
        return false;
    }
    unsafe { SetWindowTextW(state.hint, wide(status).as_ptr()) };
    true
}

fn show_answer_window(mut init: AnswerInit) -> Option<HWND> {
    if !*REGISTERED.get_or_init(register_window_class) {
        return None;
    }
    let dpi = unsafe { GetDpiForWindow(init.owner) }.max(96);
    let width = scale(DEFAULT_WINDOW_WIDTH, dpi);
    let height = scale(DEFAULT_WINDOW_HEIGHT, dpi);
    let mut owner_rect: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetWindowRect(init.owner, &mut owner_rect) };
    let x = owner_rect.left + (owner_rect.right - owner_rect.left - width) / 2;
    let y = owner_rect.top;
    let window = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            wide(CLASS_NAME).as_ptr(),
            wide("QuickSpark AI 回答").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            x,
            y,
            width,
            height,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            &mut init as *mut AnswerInit as _,
        )
    };
    if window.is_null() {
        return None;
    }
    let dark_mode = i32::from(!init.light_theme);
    unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &dark_mode as *const _ as _,
            std::mem::size_of_val(&dark_mode) as u32,
        );
    }
    unsafe {
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
    }
    Some(window)
}

pub(crate) fn append_ai_reasoning(window: HWND, text: &str) -> bool {
    if window.is_null() || unsafe { IsWindow(window) } == 0 {
        return false;
    }
    let Some(state) = answer_state!(window) else {
        return false;
    };
    if !state.streaming || state.thinking_complete || text.is_empty() {
        return false;
    }
    if state.reasoning.is_empty() {
        state.thinking_started.get_or_insert_with(Instant::now);
        state.thinking_visible = true;
    }
    state.reasoning.push_str(text);
    state.reasoning_chars = state.reasoning_chars.saturating_add(text.chars().count());
    unsafe {
        schedule_thinking_render(window, state);
        SetWindowTextW(state.hint, wide("正在思考…").as_ptr());
        EnableWindow(state.thinking_toggle, 1);
        EnableWindow(state.thinking_copy, 1);
    }
    true
}

pub(crate) fn append_ai_answer(window: HWND, text: &str) -> bool {
    if window.is_null() || unsafe { IsWindow(window) } == 0 {
        return false;
    }
    let Some(state) = answer_state!(window) else {
        return false;
    };
    if !state.streaming {
        return false;
    }
    unsafe { complete_thinking(window, state) };
    state.answer.push_str(text);
    unsafe {
        schedule_render(window, state);
        SetWindowTextW(state.hint, wide("正在回答…").as_ptr());
        SetWindowTextW(state.copy_button, wide("复制回答").as_ptr());
    }
    true
}

pub(crate) fn finish_ai_answer(window: HWND, finish_reason: Option<&str>) {
    let status = match finish_reason {
        Some("length") => "输出达到长度上限，回答可能不完整",
        Some("content_filter") => "输出因内容过滤而结束",
        Some(reason) if reason != "stop" => "生成结束（接口返回非标准原因）",
        _ => "生成完成",
    };
    unsafe { set_stream_state(window, status) };
}

pub(crate) fn fail_ai_answer(window: HWND, error: &str) {
    if window.is_null() || unsafe { IsWindow(window) } == 0 {
        return;
    }
    let Some(state) = answer_state!(window) else {
        return;
    };
    let message = if state.answer.is_empty() {
        format!("请求失败：{error}")
    } else {
        format!("\n\n[请求中断] {error}")
    };
    append_ai_answer(window, &message);
    unsafe { set_stream_state(window, "请求失败") };
}

pub(crate) fn cancel_ai_answer(window: HWND) {
    unsafe { set_stream_state(window, "已停止生成") };
}

pub(crate) fn close_ai_answer(window: HWND) {
    if !window.is_null() && unsafe { IsWindow(window) } != 0 {
        unsafe { DestroyWindow(window) };
    }
}

fn register_window_class() -> bool {
    let instance: HINSTANCE = unsafe { GetModuleHandleW(null()) };
    let class_name = wide(CLASS_NAME);
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(answer_window_proc),
        cbWndExtra: std::mem::size_of::<isize>() as i32,
        hInstance: instance,
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: null_mut(),
        lpszClassName: class_name.as_ptr(),
        ..Default::default()
    };
    unsafe { RegisterClassW(&class) != 0 }
}

unsafe extern "system" fn answer_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CREATE => unsafe { create_answer_window(window, lparam) },
        WM_SIZE => {
            if let Some(state) = answer_state!(window) {
                unsafe { layout(window, state) };
            }
            0
        }
        WM_GETMINMAXINFO => {
            let dpi = unsafe { GetDpiForWindow(window) }.max(96);
            let limits = unsafe { &mut *(lparam as *mut MINMAXINFO) };
            limits.ptMinTrackSize.x = scale(640, dpi);
            limits.ptMinTrackSize.y = scale(520, dpi);
            0
        }
        WM_DRAWITEM => {
            if let Some(state) = answer_state!(window) {
                let draw = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
                unsafe { draw_button(draw, state) };
                return 1;
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_SETFOCUS => {
            if let Some(state) = answer_state!(window) {
                unsafe { SetFocus(state.answer_edit) };
            }
            0
        }
        WM_TIMER if wparam == RENDER_TIMER_ID => {
            if let Some(state) = answer_state!(window) {
                unsafe { flush_render(window, state) };
            }
            0
        }
        WM_TIMER if wparam == THINK_RENDER_TIMER_ID => {
            if let Some(state) = answer_state!(window) {
                unsafe { flush_thinking_render(window, state) };
            }
            0
        }
        WM_COMMAND => {
            let command = wparam & 0xffff;
            if command == COPY_BUTTON_ID {
                if let Some(state) = answer_state!(window) {
                    if copy_text(window, &state.answer) {
                        unsafe {
                            SetWindowTextW(state.copy_button, wide("已复制").as_ptr());
                            SetFocus(state.answer_edit);
                        }
                    } else {
                        unsafe {
                            MessageBoxW(
                                window,
                                wide("无法复制 AI 回答。").as_ptr(),
                                wide("QuickSpark AI").as_ptr(),
                                MB_OK | MB_ICONERROR,
                            )
                        };
                    }
                }
                return 0;
            }
            if command == COPY_THINKING_ID {
                if let Some(state) = answer_state!(window)
                    && !state.reasoning.is_empty()
                {
                    if copy_text(window, &state.reasoning) {
                        unsafe {
                            SetWindowTextW(state.thinking_copy, wide("已复制").as_ptr());
                            SetFocus(state.answer_edit);
                        }
                    } else {
                        unsafe {
                            MessageBoxW(
                                window,
                                wide("无法复制完整思考过程。").as_ptr(),
                                wide("QuickSpark AI").as_ptr(),
                                MB_OK | MB_ICONERROR,
                            )
                        };
                    }
                }
                return 0;
            }
            if command == THINK_TOGGLE_ID {
                if let Some(state) = answer_state!(window)
                    && !state.reasoning.is_empty()
                {
                    unsafe { toggle_thinking(window, state) };
                }
                return 0;
            }
            if command == STOP_BUTTON_ID {
                unsafe { notify_cancel(window) };
                return 0;
            }
            if command == CLOSE_BUTTON_ID {
                unsafe { notify_cancel(window) };
                unsafe { DestroyWindow(window) };
                return 0;
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            if let Some(state) = answer_state!(window) {
                let control = lparam as HWND;
                let dc = wparam as HDC;
                if control == state.answer_edit || control == state.thinking_edit {
                    unsafe {
                        SetTextColor(
                            dc,
                            if control == state.thinking_edit {
                                state.colors.dim
                            } else {
                                state.colors.text
                            },
                        );
                        SetBkColor(dc, state.colors.answer_bg);
                    }
                    return state.answer_brush as LRESULT;
                }
                unsafe {
                    SetBkMode(dc, TRANSPARENT as i32);
                    SetTextColor(
                        dc,
                        if control == state.hint {
                            state.colors.dim
                        } else {
                            state.colors.text
                        },
                    );
                }
                return state.window_brush as LRESULT;
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_CTLCOLORBTN => {
            if let Some(state) = answer_state!(window) {
                let dc = wparam as HDC;
                unsafe {
                    SetBkMode(dc, TRANSPARENT as i32);
                    SetTextColor(dc, state.colors.text);
                    SetBkColor(dc, state.colors.bg);
                }
                return state.window_brush as LRESULT;
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_ERASEBKGND => {
            if let Some(state) = answer_state!(window) {
                let mut rect: RECT = unsafe { std::mem::zeroed() };
                unsafe {
                    GetClientRect(window, &mut rect);
                    FillRect(wparam as HDC, &rect, state.window_brush);
                }
                return 1;
            }
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_CLOSE => {
            unsafe { notify_cancel(window) };
            unsafe { DestroyWindow(window) };
            0
        }
        WM_DESTROY => {
            unsafe { KillTimer(window, RENDER_TIMER_ID) };
            unsafe { KillTimer(window, THINK_RENDER_TIMER_ID) };
            0
        }
        WM_NCDESTROY => {
            let pointer = unsafe { GetWindowLongPtrW(window, 0) } as *mut AnswerWindow;
            if !pointer.is_null() {
                // Detach the pointer first so no re-entrant teardown message can
                // access state after it has been freed.
                unsafe { SetWindowLongPtrW(window, 0, 0) };
                let state = unsafe { Box::from_raw(pointer) };
                let owner = state.owner;
                let closed_message = state.closed_message;
                let generation = state.generation;
                unsafe {
                    DeleteObject(state.heading_font as _);
                    DeleteObject(state.body_font as _);
                    DeleteObject(state.hint_font as _);
                    DeleteObject(state.answer_font as _);
                    DeleteObject(state.window_brush as _);
                    DeleteObject(state.answer_brush as _);
                }
                if closed_message != 0 && !owner.is_null() {
                    unsafe {
                        PostMessageW(
                            owner,
                            closed_message,
                            generation as WPARAM,
                            window as LPARAM,
                        )
                    };
                }
            }
            0
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

unsafe fn create_answer_window(window: HWND, lparam: LPARAM) -> LRESULT {
    let create = unsafe { &*(lparam as *const CREATESTRUCTW) };
    let init = unsafe { &*(create.lpCreateParams as *const AnswerInit) };
    let dpi = unsafe { GetDpiForWindow(window) }.max(96);
    let heading_font = create_font(dpi, 17, FW_SEMIBOLD);
    let body_font = create_font(dpi, ANSWER_FONT_PIXELS, FW_NORMAL);
    let hint_font = create_font(dpi, 14, FW_NORMAL);
    let answer_font = create_font(dpi, ANSWER_FONT_PIXELS, FW_NORMAL);
    let question_label = unsafe { create_static(window, "问题") };
    let prompt = unsafe { create_static(window, &init.prompt) };
    let answer_label = unsafe { create_static(window, "回答") };
    let (answer_edit, rich_edit) = unsafe { create_answer_edit(window) };
    let thinking_toggle =
        unsafe { create_button(window, "思考过程 · 等待模型输出", THINK_TOGGLE_ID) };
    let thinking_copy = unsafe { create_button(window, "复制思考", COPY_THINKING_ID) };
    let (thinking_edit, thinking_rich_edit) = unsafe { create_answer_edit(window) };
    let hint_text = if init.streaming {
        "正在连接 AI 接口…"
    } else {
        "生成完成 · Ctrl+A 全选 · Ctrl+C 复制"
    };
    let hint = unsafe { create_static(window, hint_text) };
    let copy_button = unsafe { create_button(window, "复制回答", COPY_BUTTON_ID) };
    let stop_button = unsafe { create_button(window, "停止生成", STOP_BUTTON_ID) };
    let close_button = unsafe { create_button(window, "关闭", CLOSE_BUTTON_ID) };
    if [
        question_label,
        prompt,
        answer_label,
        answer_edit,
        thinking_toggle,
        thinking_copy,
        thinking_edit,
        hint,
        copy_button,
        stop_button,
        close_button,
    ]
    .iter()
    .any(|control| control.is_null())
        || heading_font.is_null()
        || body_font.is_null()
        || hint_font.is_null()
        || answer_font.is_null()
    {
        unsafe {
            delete_font(heading_font);
            delete_font(body_font);
            delete_font(hint_font);
            delete_font(answer_font);
        }
        return -1;
    }
    if !init.light_theme {
        let dark_control_theme = wide("DarkMode_Explorer");
        for control in [
            answer_edit,
            thinking_edit,
            thinking_toggle,
            thinking_copy,
            copy_button,
            stop_button,
            close_button,
        ] {
            unsafe { SetWindowTheme(control, dark_control_theme.as_ptr(), null()) };
        }
    }
    let reasoning = init.reasoning.replace("\r\n", "\n");
    let reasoning_chars = reasoning.chars().count();
    let thinking_visible = init.streaming || !reasoning.is_empty();
    let colors = answer_colors(init.light_theme);
    let state = Box::new(AnswerWindow {
        question_label,
        prompt,
        answer_label,
        answer_edit,
        thinking_toggle,
        thinking_copy,
        thinking_edit,
        hint,
        copy_button,
        stop_button,
        close_button,
        heading_font,
        body_font,
        hint_font,
        answer_font,
        window_brush: unsafe { CreateSolidBrush(colors.bg) },
        answer_brush: unsafe { CreateSolidBrush(colors.answer_bg) },
        colors,
        answer: init.answer.replace("\r\n", "\n"),
        reasoning,
        reasoning_chars,
        rich_edit,
        thinking_rich_edit,
        render_dirty: false,
        thinking_dirty: false,
        thinking_visible,
        thinking_expanded: false,
        thinking_complete: !init.streaming,
        thinking_started: init.streaming.then(Instant::now),
        thinking_elapsed_ms: None,
        owner: init.owner,
        cancel_message: init.cancel_message,
        closed_message: init.closed_message,
        generation: init.generation,
        streaming: init.streaming,
    });
    if state.window_brush.is_null() || state.answer_brush.is_null() {
        unsafe {
            DeleteObject(state.heading_font as _);
            DeleteObject(state.body_font as _);
            DeleteObject(state.hint_font as _);
            DeleteObject(state.answer_font as _);
            if !state.window_brush.is_null() {
                DeleteObject(state.window_brush as _);
            }
            if !state.answer_brush.is_null() {
                DeleteObject(state.answer_brush as _);
            }
        }
        return -1;
    }
    let pointer = Box::into_raw(state);
    unsafe { SetWindowLongPtrW(window, 0, pointer as isize) };
    let state = unsafe { &mut *pointer };
    unsafe {
        for control in [state.question_label, state.answer_label] {
            SendMessageW(control, WM_SETFONT, state.heading_font as WPARAM, 1);
        }
        SendMessageW(state.prompt, WM_SETFONT, state.body_font as WPARAM, 1);
        SendMessageW(
            state.answer_edit,
            WM_SETFONT,
            state.answer_font as WPARAM,
            1,
        );
        for control in [
            state.hint,
            state.copy_button,
            state.stop_button,
            state.close_button,
            state.thinking_toggle,
            state.thinking_copy,
            state.thinking_edit,
        ] {
            SendMessageW(control, WM_SETFONT, state.hint_font as WPARAM, 1);
        }
        EnableWindow(state.stop_button, i32::from(state.streaming));
        ShowWindow(
            state.stop_button,
            if state.streaming { SW_SHOW } else { SW_HIDE },
        );
        EnableWindow(
            state.thinking_toggle,
            i32::from(!state.reasoning.is_empty()),
        );
        EnableWindow(state.thinking_copy, i32::from(!state.reasoning.is_empty()));
        ShowWindow(
            state.thinking_toggle,
            if state.thinking_visible {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        ShowWindow(state.thinking_copy, SW_HIDE);
        ShowWindow(state.thinking_edit, SW_HIDE);
        let horizontal_margin = scale(12, dpi) as u32;
        for edit in [state.answer_edit, state.thinking_edit] {
            SendMessageW(
                edit,
                EM_SETMARGINS,
                EC_LEFTMARGIN | EC_RIGHTMARGIN,
                (horizontal_margin | (horizontal_margin << 16)) as LPARAM,
            );
        }
        render_answer(state);
        update_thinking_header(state);
        SetWindowSubclass(
            state.answer_edit,
            Some(answer_edit_proc),
            EDIT_SUBCLASS_ID,
            window as usize,
        );
        SetWindowSubclass(
            state.thinking_edit,
            Some(answer_edit_proc),
            THINK_EDIT_SUBCLASS_ID,
            window as usize,
        );
        for button in [
            state.thinking_toggle,
            state.thinking_copy,
            state.copy_button,
            state.stop_button,
            state.close_button,
        ] {
            SetWindowSubclass(
                button,
                Some(answer_edit_proc),
                BUTTON_SUBCLASS_ID,
                window as usize,
            );
        }
        layout(window, state);
        SetFocus(state.answer_edit);
    }
    0
}

unsafe extern "system" fn answer_edit_proc(
    edit: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    parent: usize,
) -> LRESULT {
    if message == WM_KEYDOWN {
        if wparam as u16 == VK_TAB {
            let backwards = unsafe { GetKeyState(VK_SHIFT as i32) as u16 & 0x8000 != 0 };
            let next = unsafe { GetNextDlgTabItem(parent as HWND, edit, i32::from(backwards)) };
            if !next.is_null() {
                unsafe { SetFocus(next) };
            }
            return 0;
        }
        if wparam as u16 == VK_RETURN && subclass_id == BUTTON_SUBCLASS_ID {
            unsafe { SendMessageW(edit, BM_CLICK, 0, 0) };
            return 0;
        }
        let control = unsafe { GetKeyState(VK_CONTROL as i32) as u16 & 0x8000 != 0 };
        if control && matches!(wparam as u32, 0x41 | 0x61) {
            unsafe { SendMessageW(edit, EM_SETSEL, 0, -1) };
            return 0;
        }
        if control && matches!(wparam as u32, 0x43 | 0x63) {
            unsafe { SendMessageW(edit, WM_COPY, 0, 0) };
            return 0;
        }
        if wparam as u16 == VK_ESCAPE {
            unsafe { SendMessageW(parent as HWND, WM_CLOSE, 0, 0) };
            return 0;
        }
    }
    if message == WM_NCDESTROY {
        unsafe { RemoveWindowSubclass(edit, Some(answer_edit_proc), subclass_id) };
    }
    unsafe { DefSubclassProc(edit, message, wparam, lparam) }
}

unsafe fn layout(window: HWND, state: &AnswerWindow) {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetClientRect(window, &mut rect) };
    let dpi = unsafe { GetDpiForWindow(window) }.max(96);
    let width = rect.right.max(1);
    let height = rect.bottom.max(1);
    let margin = scale(22, dpi);
    let question_top = scale(17, dpi);
    let heading_height = scale(25, dpi);
    let prompt_top = question_top + heading_height;
    let prompt_height = scale(52, dpi);
    let footer_height = scale(56, dpi);
    let answer_bottom = height - footer_height;
    let thinking_top = prompt_top + prompt_height + scale(7, dpi);
    let thinking_row_height = scale(32, dpi);
    let thinking_copy_width = scale(98, dpi);
    let control_gap = scale(8, dpi);
    let mut answer_heading_top = prompt_top + prompt_height + scale(9, dpi);
    let mut thinking_edit_top = 0;
    let mut thinking_edit_height = 0;
    if state.thinking_visible {
        answer_heading_top = thinking_top + thinking_row_height + scale(8, dpi);
        if state.thinking_expanded {
            thinking_edit_top = thinking_top + thinking_row_height + scale(5, dpi);
            let available = (answer_bottom - thinking_edit_top).max(scale(180, dpi));
            thinking_edit_height = (available * 38 / 100).clamp(scale(110, dpi), scale(210, dpi));
            answer_heading_top = thinking_edit_top + thinking_edit_height + scale(8, dpi);
        }
    }
    let answer_top = answer_heading_top + heading_height + scale(5, dpi);
    unsafe {
        MoveWindow(
            state.question_label,
            margin,
            question_top,
            width - margin * 2,
            heading_height,
            1,
        );
        MoveWindow(
            state.prompt,
            margin,
            prompt_top,
            width - margin * 2,
            prompt_height,
            1,
        );
        MoveWindow(
            state.thinking_toggle,
            margin,
            thinking_top,
            (width - margin * 2 - thinking_copy_width - control_gap).min(scale(360, dpi)),
            thinking_row_height,
            1,
        );
        MoveWindow(
            state.thinking_copy,
            width - margin - thinking_copy_width,
            thinking_top,
            thinking_copy_width,
            thinking_row_height,
            1,
        );
        MoveWindow(
            state.thinking_edit,
            margin,
            thinking_edit_top,
            width - margin * 2,
            thinking_edit_height,
            1,
        );
        MoveWindow(
            state.answer_label,
            margin,
            answer_heading_top,
            width - margin * 2,
            heading_height,
            1,
        );
        MoveWindow(
            state.answer_edit,
            margin,
            answer_top,
            width - margin * 2,
            (answer_bottom - answer_top).max(scale(80, dpi)),
            1,
        );
        for edit in std::iter::once(state.answer_edit)
            .chain(state.thinking_expanded.then_some(state.thinking_edit))
        {
            let mut edit_rect: RECT = std::mem::zeroed();
            GetClientRect(edit, &mut edit_rect);
            let inset = scale(12, dpi);
            edit_rect.left += inset;
            edit_rect.top += inset;
            edit_rect.right -= inset;
            edit_rect.bottom -= inset;
            SendMessageW(edit, EM_SETRECT, 0, &edit_rect as *const RECT as LPARAM);
        }
        let button_height = scale(32, dpi);
        let close_width = scale(82, dpi);
        let copy_width = scale(108, dpi);
        let stop_width = scale(108, dpi);
        let button_gap = scale(10, dpi);
        let footer_top = height - scale(44, dpi);
        MoveWindow(
            state.hint,
            margin,
            footer_top + scale(6, dpi),
            width
                - margin * 2
                - copy_width
                - close_width
                - button_gap * 2
                - if state.streaming {
                    stop_width + button_gap
                } else {
                    0
                },
            scale(24, dpi),
            1,
        );
        MoveWindow(
            state.stop_button,
            width - margin - close_width - copy_width - stop_width - button_gap * 2,
            footer_top,
            stop_width,
            button_height,
            1,
        );
        MoveWindow(
            state.copy_button,
            width - margin - close_width - copy_width - button_gap,
            footer_top,
            copy_width,
            button_height,
            1,
        );
        MoveWindow(
            state.close_button,
            width - margin - close_width,
            footer_top,
            close_width,
            button_height,
            1,
        );
    }
}

unsafe fn notify_cancel(window: HWND) {
    let Some(state) = answer_state!(window) else {
        return;
    };
    if !state.streaming {
        return;
    }
    unsafe { complete_thinking(window, state) };
    state.streaming = false;
    state.render_dirty = true;
    unsafe { flush_render(window, state) };
    unsafe {
        if GetFocus() == state.stop_button {
            SetFocus(state.answer_edit);
        }
        EnableWindow(state.stop_button, 0);
        ShowWindow(state.stop_button, SW_HIDE);
        SetWindowTextW(state.hint, wide("正在停止生成…").as_ptr());
        layout(window, state);
        PostMessageW(
            state.owner,
            state.cancel_message,
            state.generation as WPARAM,
            window as LPARAM,
        );
    }
}

unsafe fn set_stream_state(window: HWND, status: &str) {
    if window.is_null() || unsafe { IsWindow(window) } == 0 {
        return;
    }
    let Some(state) = answer_state!(window) else {
        return;
    };
    unsafe { complete_thinking(window, state) };
    state.streaming = false;
    // The last streaming paint is intentionally plain text. Always perform one
    // final Markdown pass, even if no delta arrived since the previous timer.
    state.render_dirty = true;
    unsafe { flush_render(window, state) };
    unsafe {
        SetWindowTextW(state.hint, wide(status).as_ptr());
        if GetFocus() == state.stop_button {
            SetFocus(state.answer_edit);
        }
        EnableWindow(state.stop_button, 0);
        ShowWindow(state.stop_button, SW_HIDE);
        layout(window, state);
    }
}

unsafe fn schedule_render(window: HWND, state: &mut AnswerWindow) {
    if state.render_dirty {
        return;
    }
    state.render_dirty = true;
    let interval = if state.answer.len() > 100_000 {
        250
    } else if state.answer.len() > 20_000 {
        100
    } else {
        RENDER_INTERVAL_MS
    };
    if unsafe { SetTimer(window, RENDER_TIMER_ID, interval, None) } == 0 {
        unsafe { flush_render(window, state) };
    }
}

unsafe fn flush_render(window: HWND, state: &mut AnswerWindow) {
    if !state.render_dirty {
        return;
    }
    unsafe { KillTimer(window, RENDER_TIMER_ID) };
    state.render_dirty = false;
    if state.streaming {
        unsafe { render_plain_answer(state) };
    } else {
        unsafe { render_answer(state) };
    }
}

unsafe fn schedule_thinking_render(window: HWND, state: &mut AnswerWindow) {
    if state.thinking_dirty {
        return;
    }
    state.thinking_dirty = true;
    if unsafe {
        SetTimer(
            window,
            THINK_RENDER_TIMER_ID,
            THINK_RENDER_INTERVAL_MS,
            None,
        )
    } == 0
    {
        unsafe { flush_thinking_render(window, state) };
    }
}

unsafe fn flush_thinking_render(window: HWND, state: &mut AnswerWindow) {
    unsafe { KillTimer(window, THINK_RENDER_TIMER_ID) };
    state.thinking_dirty = false;
    unsafe {
        update_thinking_header(state);
        SetWindowTextW(state.thinking_copy, wide("复制思考").as_ptr());
    }
    if state.thinking_expanded {
        unsafe { render_thinking(state) };
    }
}

unsafe fn complete_thinking(window: HWND, state: &mut AnswerWindow) {
    if state.thinking_complete {
        return;
    }
    state.thinking_complete = true;
    state.thinking_elapsed_ms = state
        .thinking_started
        .map(|started| started.elapsed().as_millis());
    unsafe { flush_thinking_render(window, state) };
    if state.reasoning.is_empty() {
        state.thinking_visible = false;
        state.thinking_expanded = false;
        unsafe {
            ShowWindow(state.thinking_toggle, SW_HIDE);
            ShowWindow(state.thinking_copy, SW_HIDE);
            ShowWindow(state.thinking_edit, SW_HIDE);
        }
    } else {
        state.thinking_visible = true;
        unsafe { update_thinking_header(state) };
    }
    unsafe { layout(window, state) };
}

unsafe fn toggle_thinking(window: HWND, state: &mut AnswerWindow) {
    state.thinking_expanded = !state.thinking_expanded;
    unsafe {
        ShowWindow(
            state.thinking_edit,
            if state.thinking_expanded {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        ShowWindow(
            state.thinking_copy,
            if state.thinking_expanded {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        if state.thinking_expanded {
            render_thinking(state);
        }
        update_thinking_header(state);
        layout(window, state);
        SetFocus(if state.thinking_expanded {
            state.thinking_edit
        } else {
            state.answer_edit
        });
    }
}

unsafe fn update_thinking_header(state: &AnswerWindow) {
    let label = if state.reasoning.is_empty() {
        "等待思考过程…".to_owned()
    } else if !state.thinking_expanded {
        if state.thinking_complete {
            "展开思考过程".to_owned()
        } else {
            "正在思考… · 展开".to_owned()
        }
    } else if state.thinking_complete {
        state.thinking_elapsed_ms.map_or_else(
            || format!("收起思考过程 · {} 字", state.reasoning_chars),
            |elapsed| {
                format!(
                    "收起思考过程 · {} 字 · {:.1} 秒",
                    state.reasoning_chars,
                    elapsed as f64 / 1_000.0
                )
            },
        )
    } else {
        format!("正在思考… {} 字 · 收起", state.reasoning_chars)
    };
    unsafe { SetWindowTextW(state.thinking_toggle, wide(&label).as_ptr()) };
}

unsafe fn create_answer_edit(parent: HWND) -> (HWND, bool) {
    const ES_NOHIDESEL: u32 = 0x0100;
    let style = WS_CHILD
        | WS_VISIBLE
        | WS_TABSTOP
        | WS_VSCROLL
        | ES_MULTILINE as u32
        | ES_AUTOVSCROLL as u32
        | ES_READONLY as u32
        | ES_NOHIDESEL;
    if rich_edit_available() {
        let edit = unsafe {
            CreateWindowExW(
                0,
                wide("RICHEDIT50W").as_ptr(),
                wide("").as_ptr(),
                style,
                0,
                0,
                0,
                0,
                parent,
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        if !edit.is_null() {
            return (edit, true);
        }
    }
    let edit = unsafe {
        CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            style,
            0,
            0,
            0,
            0,
            parent,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    };
    (edit, false)
}

unsafe fn answer_window_ptr(window: HWND) -> *mut AnswerWindow {
    unsafe { GetWindowLongPtrW(window, 0) as *mut AnswerWindow }
}

unsafe fn create_static(parent: HWND, text: &str) -> HWND {
    unsafe {
        CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            parent,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        )
    }
}

unsafe fn create_button(parent: HWND, text: &str, id: usize) -> HWND {
    unsafe {
        CreateWindowExW(
            0,
            wide("BUTTON").as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            0,
            0,
            0,
            0,
            parent,
            id as _,
            GetModuleHandleW(null()),
            null(),
        )
    }
}

// Keep native button semantics while matching the launcher's flat surfaces.
unsafe fn draw_button(draw: &DRAWITEMSTRUCT, state: &AnswerWindow) {
    let dpi = unsafe { GetDpiForWindow(draw.hwndItem) }.max(96);
    let quiet = matches!(draw.CtlID as usize, THINK_TOGGLE_ID | COPY_THINKING_ID);
    let disabled = draw.itemState & ODS_DISABLED != 0;
    let pressed = draw.itemState & ODS_SELECTED != 0;
    let primary = draw.CtlID as usize == COPY_BUTTON_ID;
    let background = if !disabled && (pressed || primary) {
        state.colors.selected
    } else {
        state.colors.bg
    };
    unsafe { FillRect(draw.hDC, &draw.rcItem, state.window_brush) };
    if !quiet || pressed {
        let brush = unsafe { CreateSolidBrush(background) };
        let pen = unsafe {
            CreatePen(
                PS_SOLID,
                1,
                if primary || quiet {
                    background
                } else {
                    state.colors.divider
                },
            )
        };
        let old_brush = unsafe { SelectObject(draw.hDC, brush as _) };
        let old_pen = unsafe { SelectObject(draw.hDC, pen as _) };
        unsafe {
            RoundRect(
                draw.hDC,
                draw.rcItem.left,
                draw.rcItem.top,
                draw.rcItem.right,
                draw.rcItem.bottom,
                scale(8, dpi),
                scale(8, dpi),
            );
            SelectObject(draw.hDC, old_pen);
            SelectObject(draw.hDC, old_brush);
            DeleteObject(pen as _);
            DeleteObject(brush as _);
        }
    }
    let length = unsafe { GetWindowTextLengthW(draw.hwndItem) };
    let mut text = vec![0u16; length as usize + 1];
    let mut text_rect = draw.rcItem;
    let inset = scale(if quiet { 2 } else { 10 }, dpi);
    text_rect.left += inset;
    text_rect.right -= inset;
    let old_font = unsafe { SelectObject(draw.hDC, state.hint_font as _) };
    unsafe {
        GetWindowTextW(draw.hwndItem, text.as_mut_ptr(), text.len() as i32);
        SetBkMode(draw.hDC, TRANSPARENT as i32);
        SetTextColor(
            draw.hDC,
            if quiet || disabled {
                state.colors.dim
            } else {
                state.colors.text
            },
        );
        DrawTextW(
            draw.hDC,
            text.as_ptr(),
            -1,
            &mut text_rect,
            DT_SINGLELINE
                | DT_VCENTER
                | DT_END_ELLIPSIS
                | if draw.CtlID as usize == THINK_TOGGLE_ID {
                    DT_LEFT
                } else {
                    DT_CENTER
                },
        );
        SelectObject(draw.hDC, old_font);
        if draw.itemState & ODS_FOCUS != 0 {
            let mut focus = draw.rcItem;
            let inset = scale(3, dpi);
            focus.left += inset;
            focus.top += inset;
            focus.right -= inset;
            focus.bottom -= inset;
            DrawFocusRect(draw.hDC, &focus);
        }
    }
}

fn create_font(dpi: u32, pixels: i32, weight: u32) -> HFONT {
    unsafe {
        CreateFontW(
            scale(-pixels, dpi),
            0,
            0,
            0,
            weight as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            FF_DONTCARE as u32,
            wide("Microsoft YaHei UI").as_ptr(),
        )
    }
}

unsafe fn delete_font(font: HFONT) {
    if !font.is_null() {
        unsafe { DeleteObject(font as _) };
    }
}

fn normalize_newlines(value: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

fn scale(value: i32, dpi: u32) -> i32 {
    value * dpi as i32 / 96
}

#[cfg(test)]
mod tests;
