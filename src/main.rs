#![windows_subsystem = "windows"]
#![deny(unsafe_op_in_unsafe_fn)]

mod model;
mod platform;
mod providers;
mod search;
mod storage;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    os::windows::ffi::OsStrExt,
    ptr::{NonNull, null, null_mut},
    sync::{
        OnceLock,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, RECT,
        WPARAM,
    },
    Graphics::{
        Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute},
        Gdi::{
            BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontW,
            CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DT_END_ELLIPSIS, DT_LEFT, DT_RIGHT,
            DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FF_DONTCARE, FW_NORMAL,
            FillRect, GetMonitorInfoW, GetStockObject, HBRUSH, HFONT, InvalidateRect, LineTo,
            MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MoveToEx, NULL_PEN,
            OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, RoundRect, SelectObject, SetBkColor,
            SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
        },
    },
    System::{
        LibraryLoader::GetModuleHandleW, Power::SetSuspendState, Shutdown::LockWorkStation,
        Threading::CreateMutexW,
    },
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_SELECTED},
        HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, GetDpiForSystem,
            GetDpiForWindow, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
        },
        Input::KeyboardAndMouse::{
            GetKeyState, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, SetFocus,
            UnregisterHotKey, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_F1, VK_RETURN, VK_SHIFT, VK_TAB,
            VK_UP,
        },
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
            Shell_NotifyIconW,
        },
        WindowsAndMessaging::*,
    },
};

use model::{Action, Query, ResultItem, SettingChange, SystemCommand, Usage};
use platform::windows::{
    ApiKeyPromptResult, activate_window, append_ai_answer, append_ai_reasoning, cancel_ai_answer,
    close_ai_answer, copy_text, fail_ai_answer, finish_ai_answer, prompt_ai_api_key, shell_execute,
    show_ai_answer, show_ai_stream, windows_program,
};
use providers::{
    AiEvent, AiService, ApplicationProvider, CalculatorProvider, CommandProvider,
    ConverterProvider, FileSearchProvider, HelpProvider, IndexError, ProviderContext,
    ProviderRegistry, SettingsProvider, SystemProvider, WebSearchProvider, WindowSwitcherProvider,
};
use storage::{
    HotkeyChoice, Settings, load_light_theme, load_pinned, load_settings, load_usage,
    migrate_legacy_data, save_pinned, save_settings, save_theme, save_usage, set_autostart,
};

const HOTKEY_ID: i32 = 1;
const EDIT_ID: i32 = 1001;
const LIST_ID: i32 = 1002;
const EM_SETMARGINS: u32 = 0x00D3;
const EM_SETCUEBANNER: u32 = 0x1501;
const EM_SETSEL: u32 = 0x00B1;
const TRAY_MESSAGE: u32 = WM_USER + 1;
const MENU_SHOW: usize = 2001;
const MENU_EXIT: usize = 2002;
const MENU_LIGHT: usize = 2003;
const MENU_REFRESH: usize = 2004;
const MENU_HELP: usize = 2005;
const MENU_SETTINGS: usize = 2006;
const MENU_ACTION_BASE: usize = 3000;
const INDEX_READY: u32 = WM_APP + 1;
const SHOW_EXISTING: u32 = WM_APP + 2;
const FILE_SEARCH_READY: u32 = WM_APP + 3;
const AI_READY: u32 = WM_APP + 4;
const AI_CANCEL: u32 = WM_APP + 5;
const AI_WINDOW_CLOSED: u32 = WM_APP + 6;
const WIDTH: i32 = 620;
const HEIGHT: i32 = 306;
const VISIBLE_RESULT_COUNT: usize = 8;
const DARK_BG: u32 = 0x001E1A18;
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

type IndexResult = Result<ApplicationProvider, IndexError>;

struct App {
    edit: HWND,
    list: HWND,
    results: Vec<ResultItem>,
    providers: ProviderRegistry,
    font: HFONT,
    shortcut_font: HFONT,
    background: HBRUSH,
    panel: HBRUSH,
    dpi: u32,
    light_theme: bool,
    usage: HashMap<String, Usage>,
    pinned: HashSet<String>,
    settings: Settings,
    hotkey_registered: bool,
    ai_service: AiService,
    ai_window: HWND,
    ai_generation: u64,
    indexing: bool,
    index_tx: Sender<IndexResult>,
    index_rx: Receiver<IndexResult>,
}

struct Theme {
    bg: u32,
    text: u32,
    dim: u32,
    muted: u32,
    selected: u32,
    divider: u32,
}

fn theme(light: bool) -> Theme {
    if light {
        Theme {
            bg: 0x00F7F5F4,
            text: 0x00242120,
            dim: 0x005B5450,
            muted: 0x00867B74,
            selected: 0x00F3DDCC,
            divider: 0x00DEDAD6,
        }
    } else {
        Theme {
            bg: DARK_BG,
            text: 0x00F2EDE9,
            dim: 0x00AAA39D,
            muted: 0x00908780,
            selected: 0x0052433B,
            divider: 0x00332E2A,
        }
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn px(value: i32, dpi: u32) -> i32 {
    value * dpi as i32 / 96
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

type AppCell = RefCell<App>;

fn try_with_state<T, R>(state: &RefCell<T>, access: impl FnOnce(&T) -> R) -> Option<R> {
    let state = state.try_borrow().ok()?;
    Some(access(&state))
}

fn try_with_state_mut<T, R>(state: &RefCell<T>, access: impl FnOnce(&mut T) -> R) -> Option<R> {
    let mut state = state.try_borrow_mut().ok()?;
    Some(access(&mut state))
}

unsafe fn app_cell(hwnd: HWND) -> Option<NonNull<AppCell>> {
    NonNull::new(unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppCell })
}

unsafe fn with_app<R>(hwnd: HWND, access: impl FnOnce(&App) -> R) -> Option<R> {
    let pointer = unsafe { app_cell(hwnd) }?;
    try_with_state(unsafe { pointer.as_ref() }, access)
}

unsafe fn with_app_mut<R>(hwnd: HWND, access: impl FnOnce(&mut App) -> R) -> Option<R> {
    let pointer = unsafe { app_cell(hwnd) }?;
    try_with_state_mut(unsafe { pointer.as_ref() }, access)
}

unsafe fn has_app(hwnd: HWND) -> bool {
    unsafe { app_cell(hwnd).is_some() }
}

unsafe fn query(edit: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(edit) };
    let mut buf = vec![0u16; len as usize + 1];
    unsafe { GetWindowTextW(edit, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..len as usize])
        .trim()
        .to_owned()
}

unsafe fn set_launcher_query(hwnd: HWND, value: &str) {
    let Some(edit) = (unsafe { with_app(hwnd, |state| state.edit) }) else {
        return;
    };
    unsafe {
        SetWindowTextW(edit, wide(value).as_ptr());
        let end = value.encode_utf16().count();
        SendMessageW(edit, EM_SETSEL, end, end as LPARAM);
        SetFocus(edit);
    }
}

unsafe fn refresh(hwnd: HWND) {
    let Some(edit) = (unsafe { with_app(hwnd, |state| state.edit) }) else {
        return;
    };
    let input = unsafe { query(edit) };
    let query = Query::new(&input);
    let Some((list, titles)) = (unsafe {
        with_app_mut(hwnd, |state| {
            let context = ProviderContext {
                usage: &state.usage,
                now: now_seconds(),
                settings: &state.settings,
                pinned: &state.pinned,
                light_theme: state.light_theme,
            };
            state.results = state
                .providers
                .query(&query, &context, state.settings.result_limit);
            let titles = state
                .results
                .iter()
                .map(|result| result.title.clone())
                .collect::<Vec<_>>();
            (state.list, titles)
        })
    }) else {
        return;
    };

    unsafe { SendMessageW(list, LB_RESETCONTENT, 0, 0) };
    for title in &titles {
        let label = wide(title);
        unsafe { SendMessageW(list, LB_ADDSTRING, 0, label.as_ptr() as LPARAM) };
    }
    if !titles.is_empty() {
        unsafe { SendMessageW(list, LB_SETCURSEL, 0, 0) };
    }
}

unsafe fn launch_selected(hwnd: HWND) {
    let Some(position) = (unsafe { selected_result_position(hwnd) }) else {
        return;
    };
    unsafe { launch_result(hwnd, position) };
}

unsafe fn launch_result(hwnd: HWND, position: usize) {
    let action = unsafe {
        with_app(hwnd, |state| {
            state
                .results
                .get(position)
                .and_then(ResultItem::primary_action)
                .cloned()
        })
    }
    .flatten();
    if let Some(action) = action {
        unsafe { execute_action(hwnd, action) };
    }
}

unsafe fn launch_result_action(hwnd: HWND, position: usize, action_position: usize) {
    let action = unsafe {
        with_app(hwnd, |state| {
            state
                .results
                .get(position)
                .and_then(|result| result.actions.get(action_position))
                .cloned()
        })
    }
    .flatten();
    let Some(action) = action else {
        return;
    };

    unsafe { execute_action(hwnd, action) };
}

unsafe fn execute_action(hwnd: HWND, action: Action) {
    let mut usage_key = None;
    let succeeded = match action {
        Action::CopyText { text } => {
            let copied = copy_text(hwnd, &text);
            if !copied {
                unsafe { show_action_error(hwnd, "无法复制到剪贴板。") };
            }
            copied
        }
        Action::LaunchApplication {
            path,
            usage_key: key,
        } => {
            let target = path.to_string_lossy();
            let launched = unsafe { shell_execute(hwnd, "open", &target, None) };
            if !launched {
                unsafe {
                    show_action_error(hwnd, "无法启动该程序，快捷方式可能已经失效。")
                };
            } else {
                usage_key = Some(key);
            }
            launched
        }
        Action::LaunchPackagedApplication {
            app_id,
            usage_key: key,
        } => {
            let parameters = format!("shell:AppsFolder\\{app_id}");
            let launched = windows_program("explorer.exe").is_some_and(|explorer| unsafe {
                shell_execute(hwnd, "open", &explorer, Some(&parameters))
            });
            if !launched {
                unsafe { show_action_error(hwnd, "无法启动该系统应用。") };
            } else {
                usage_key = Some(key);
            }
            launched
        }
        Action::RunAsAdministrator { path } => {
            let target = path.to_string_lossy();
            let launched = unsafe { shell_execute(hwnd, "runas", &target, None) };
            if !launched {
                unsafe { show_action_error(hwnd, "无法以管理员身份启动该程序。") };
            }
            launched
        }
        Action::OpenContainingFolder { path } => {
            let parameters = format!("/select,\"{}\"", path.to_string_lossy());
            let opened = windows_program("explorer.exe").is_some_and(|explorer| unsafe {
                shell_execute(hwnd, "open", &explorer, Some(&parameters))
            });
            if !opened {
                unsafe { show_action_error(hwnd, "无法打开程序所在位置。") };
            }
            opened
        }
        Action::OpenPath { path } => {
            let target = path.to_string_lossy();
            let opened = unsafe { shell_execute(hwnd, "open", &target, None) };
            if !opened {
                unsafe { show_action_error(hwnd, "无法打开该文件或文件夹。") };
            }
            opened
        }
        Action::OpenUrl { url } => {
            let opened = unsafe { shell_execute(hwnd, "open", &url, None) };
            if !opened {
                unsafe { show_action_error(hwnd, "无法打开默认浏览器。") };
            }
            opened
        }
        Action::OpenTarget { target } => {
            let opened = unsafe { shell_execute(hwnd, "open", &target, None) };
            if !opened {
                unsafe { show_action_error(hwnd, "无法打开该目标。") };
            }
            opened
        }
        Action::ActivateWindow { handle } => {
            let activated = activate_window(handle);
            if !activated {
                unsafe { show_action_error(hwnd, "该窗口已经关闭或无法切换。") };
            }
            activated
        }
        Action::SetQuery { query } => {
            unsafe { set_launcher_query(hwnd, &query) };
            return;
        }
        Action::SetPinned { key, pinned } => {
            let Some(snapshot) = (unsafe {
                with_app_mut(hwnd, |state| {
                    if pinned {
                        state.pinned.insert(key.clone());
                    } else {
                        state.pinned.remove(&key);
                    }
                    state.pinned.clone()
                })
            }) else {
                return;
            };
            if !save_pinned(&snapshot) {
                let _ = unsafe {
                    with_app_mut(hwnd, |state| {
                        if pinned {
                            state.pinned.remove(&key);
                        } else {
                            state.pinned.insert(key);
                        }
                    })
                };
                unsafe {
                    show_action_error(hwnd, "无法保存固定项目，已撤销本次更改。")
                };
            }
            unsafe { refresh(hwnd) };
            return;
        }
        Action::ChangeSetting { change } => {
            unsafe { apply_setting_change(hwnd, change) };
            return;
        }
        Action::SystemCommand { command } => unsafe { execute_system_command(hwnd, command) },
        Action::AskAi { request } | Action::RegenerateAi { request } => {
            let Some(previous_window) = (unsafe {
                with_app_mut(hwnd, |state| {
                    state.ai_service.cancel();
                    let window = state.ai_window;
                    state.ai_window = null_mut();
                    state.ai_generation = 0;
                    window
                })
            }) else {
                return;
            };
            close_ai_answer(previous_window);
            let Some(generation) =
                (unsafe { with_app_mut(hwnd, |state| state.ai_service.ask(request.clone())) })
            else {
                return;
            };
            let Some(answer_window) = show_ai_stream(
                hwnd,
                &request.display_prompt,
                AI_CANCEL,
                AI_WINDOW_CLOSED,
                generation,
            ) else {
                let _ = unsafe { with_app_mut(hwnd, |state| state.ai_service.cancel()) };
                unsafe { show_action_error(hwnd, "无法打开 AI 回答窗口。") };
                unsafe { refresh(hwnd) };
                return;
            };
            let updated = unsafe {
                with_app_mut(hwnd, |state| {
                    state.ai_window = answer_window;
                    state.ai_generation = generation;
                })
            };
            if updated.is_none() {
                close_ai_answer(answer_window);
                return;
            }
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            unsafe { refresh(hwnd) };
            return;
        }
        Action::CancelAi => {
            let Some(answer_window) = (unsafe {
                with_app_mut(hwnd, |state| {
                    state.ai_service.cancel();
                    state.ai_window
                })
            }) else {
                return;
            };
            cancel_ai_answer(answer_window);
            unsafe { refresh(hwnd) };
            return;
        }
        Action::ShowAiAnswer {
            prompt,
            answer,
            reasoning,
        } => {
            if !show_ai_answer(hwnd, &prompt, &answer, &reasoning) {
                unsafe { show_action_error(hwnd, "无法打开 AI 回答窗口。") };
            }
            return;
        }
        Action::ConfigureAiKey => {
            match prompt_ai_api_key(hwnd) {
                Ok(ApiKeyPromptResult::Saved) => unsafe {
                    MessageBoxW(
                        hwnd,
                        wide("API Key 已安全保存到 Windows 凭据管理器。").as_ptr(),
                        wide("QuickSpark AI").as_ptr(),
                        MB_OK | MB_ICONINFORMATION,
                    );
                },
                Ok(ApiKeyPromptResult::Cleared) => unsafe {
                    MessageBoxW(
                        hwnd,
                        wide("已清除保存的 AI API Key。").as_ptr(),
                        wide("QuickSpark AI").as_ptr(),
                        MB_OK | MB_ICONINFORMATION,
                    );
                },
                Ok(ApiKeyPromptResult::Cancelled) => {}
                Err(error) => unsafe { show_action_error(hwnd, &error) },
            }
            unsafe { refresh(hwnd) };
            return;
        }
    };

    if !succeeded {
        return;
    }
    if let Some(key) = usage_key {
        let snapshot = unsafe {
            with_app_mut(hwnd, |state| {
                let record = state.usage.entry(key).or_default();
                record.count = record.count.saturating_add(1);
                record.last = now_seconds();
                state.usage.clone()
            })
        };
        if let Some(snapshot) = snapshot {
            save_usage(&snapshot);
        }
    }
    unsafe { ShowWindow(hwnd, SW_HIDE) };
}

unsafe fn show_action_error(hwnd: HWND, message: &str) {
    unsafe {
        MessageBoxW(
            hwnd,
            wide(message).as_ptr(),
            wide("QuickSpark").as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

unsafe fn selected_result_position(hwnd: HWND) -> Option<usize> {
    let list = unsafe { with_app(hwnd, |state| state.list) }?;
    let selected = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) } as i32;
    (selected >= 0).then_some(selected as usize)
}

unsafe fn launch_selected_matching(hwnd: HWND, matches: fn(&Action) -> bool) -> bool {
    let Some(position) = (unsafe { selected_result_position(hwnd) }) else {
        return false;
    };
    let action_position = unsafe {
        with_app(hwnd, |state| {
            state
                .results
                .get(position)
                .and_then(|result| result.actions.iter().position(matches))
        })
    }
    .flatten();
    let Some(action_position) = action_position else {
        return false;
    };
    unsafe { launch_result_action(hwnd, position, action_position) };
    true
}

unsafe fn show_result_actions(hwnd: HWND) {
    let Some(position) = (unsafe { selected_result_position(hwnd) }) else {
        return;
    };
    let Some((actions, dpi)) = (unsafe {
        with_app(hwnd, |state| {
            let mut actions = state
                .results
                .get(position)
                .map(|result| result.actions.clone())
                .unwrap_or_default();
            if let Some(key) = actions.first().and_then(Action::favorite_key) {
                let pinned = !state.pinned.contains(&key);
                actions.push(Action::SetPinned { key, pinned });
            }
            (actions, state.dpi)
        })
    }) else {
        return;
    };
    if actions.len() <= 1 {
        return;
    }
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    for (index, action) in actions.iter().enumerate() {
        unsafe {
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_ACTION_BASE + index,
                wide(action.menu_text()).as_ptr(),
            )
        };
    }

    let mut rect: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetWindowRect(hwnd, &mut rect) };
    let command = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            rect.right - px(260, dpi),
            rect.top + px(44, dpi),
            0,
            hwnd,
            null(),
        )
    } as usize;
    unsafe { DestroyMenu(menu) };
    if command >= MENU_ACTION_BASE
        && command < MENU_ACTION_BASE + actions.len()
        && let Some(action) = actions.get(command - MENU_ACTION_BASE).cloned()
    {
        unsafe { execute_action(hwnd, action) };
    }
}

fn modifier_pressed(key: i32) -> bool {
    unsafe { GetKeyState(key) as u16 & 0x8000 != 0 }
}

unsafe fn handle_action_shortcut(hwnd: HWND, key: u16) -> bool {
    let control = modifier_pressed(VK_CONTROL as i32);
    let shift = modifier_pressed(VK_SHIFT as i32);
    if key == VK_TAB {
        unsafe { show_result_actions(hwnd) };
        return true;
    }
    if control && key == VK_RETURN {
        return unsafe {
            launch_selected_matching(hwnd, |action| {
                matches!(action, Action::RunAsAdministrator { .. })
            })
        };
    }
    if control && shift && key == 0x45 {
        return unsafe {
            launch_selected_matching(hwnd, |action| {
                matches!(action, Action::OpenContainingFolder { .. })
            })
        };
    }
    if control && key == 0x43 {
        return unsafe {
            launch_selected_matching(hwnd, |action| matches!(action, Action::CopyText { .. }))
        };
    }
    false
}

fn alt_shortcut_index(message: u32, key: WPARAM) -> Option<usize> {
    if message == WM_SYSKEYDOWN && (0x31..=0x38).contains(&(key as u32)) {
        Some(key - 0x31)
    } else {
        None
    }
}

fn wheel_selection_delta(wparam: WPARAM) -> i32 {
    const WHEEL_DELTA: u32 = 120;
    let delta = ((wparam >> 16) as u16 as i16) as i32;
    if delta == 0 {
        return 0;
    }
    let steps = (delta.unsigned_abs() / WHEEL_DELTA).max(1) as i32;
    -delta.signum() * steps
}

unsafe fn move_selection(hwnd: HWND, delta: i32) {
    let Some(list) = (unsafe { with_app(hwnd, |state| state.list) }) else {
        return;
    };
    let count = unsafe { SendMessageW(list, LB_GETCOUNT, 0, 0) } as i32;
    if count <= 0 {
        return;
    }
    let current = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) } as i32;
    let next = (current + delta).clamp(0, count - 1);
    unsafe { SendMessageW(list, LB_SETCURSEL, next as WPARAM, 0) };
    let top = unsafe { SendMessageW(list, LB_GETTOPINDEX, 0, 0) } as i32;
    let visible_count = VISIBLE_RESULT_COUNT as i32;
    let new_top = if next < top {
        next
    } else if next >= top + visible_count {
        next - visible_count + 1
    } else {
        top
    };
    if new_top != top {
        unsafe { SendMessageW(list, LB_SETTOPINDEX, new_top as WPARAM, 0) };
    }
}

unsafe fn start_index_refresh(hwnd: HWND) {
    let sender = unsafe {
        with_app_mut(hwnd, |state| {
            if state.indexing {
                return None;
            }
            state.indexing = true;
            Some(state.index_tx.clone())
        })
    }
    .flatten();
    let Some(sender) = sender else {
        return;
    };
    let hwnd_value = hwnd as usize;
    thread::spawn(move || {
        let result = catch_index_panic(ApplicationProvider::build);
        if sender.send(result).is_ok() {
            unsafe { PostMessageW(hwnd_value as HWND, INDEX_READY, 0, 0) };
        }
    });
}

fn catch_index_panic<F>(build: F) -> IndexResult
where
    F: FnOnce() -> IndexResult + std::panic::UnwindSafe,
{
    std::panic::catch_unwind(build).unwrap_or(Err(IndexError::Panic))
}

unsafe fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    data.hIcon = unsafe { application_icon() };
    let hotkey = if unsafe { has_app(hwnd) } {
        unsafe {
            with_app(hwnd, |state| {
                if state.hotkey_registered {
                    state.settings.hotkey.label()
                } else {
                    "快捷键不可用"
                }
            })
        }
        .unwrap_or("快捷键不可用")
    } else {
        HotkeyChoice::AltSpace.label()
    };
    let tip = wide(&format!("QuickSpark · {hotkey}"));
    let count = tip.len().min(data.szTip.len());
    data.szTip[..count].copy_from_slice(&tip[..count]);
    data
}

unsafe fn application_icon() -> HICON {
    let instance = unsafe { GetModuleHandleW(null()) };
    let resource_id = std::ptr::without_provenance::<u16>(1);
    let icon = unsafe { LoadIconW(instance, resource_id) };
    if icon.is_null() {
        unsafe { LoadIconW(null_mut(), IDI_APPLICATION) }
    } else {
        icon
    }
}

unsafe fn apply_dpi(hwnd: HWND, dpi: u32) {
    if unsafe { with_app(hwnd, |state| state.dpi == dpi) }.unwrap_or(true) {
        return;
    }
    let face = wide("Microsoft YaHei UI");
    let font = unsafe {
        CreateFontW(
            px(-19, dpi),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            FF_DONTCARE as u32,
            face.as_ptr(),
        )
    };
    let shortcut_font = unsafe {
        CreateFontW(
            px(-18, dpi),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            FF_DONTCARE as u32,
            face.as_ptr(),
        )
    };
    if font.is_null() || shortcut_font.is_null() {
        if !font.is_null() {
            unsafe { DeleteObject(font as _) };
        }
        if !shortcut_font.is_null() {
            unsafe { DeleteObject(shortcut_font as _) };
        }
        return;
    }
    let swapped = unsafe {
        with_app_mut(hwnd, |state| {
            let old = (state.edit, state.list, state.font, state.shortcut_font);
            state.font = font;
            state.shortcut_font = shortcut_font;
            state.dpi = dpi;
            old
        })
    };
    let Some((edit, list, old_font, old_shortcut_font)) = swapped else {
        unsafe {
            DeleteObject(font as _);
            DeleteObject(shortcut_font as _);
        }
        return;
    };
    unsafe {
        SendMessageW(edit, WM_SETFONT, font as WPARAM, 1);
        SendMessageW(list, WM_SETFONT, font as WPARAM, 1);
        let left = px(8, dpi) as u32;
        let right = px(8, dpi) as u32;
        SendMessageW(
            edit,
            EM_SETMARGINS,
            (EC_LEFTMARGIN | EC_RIGHTMARGIN) as WPARAM,
            (left | (right << 16)) as LPARAM,
        );
        let item_height = px(32, dpi);
        SendMessageW(list, LB_SETITEMHEIGHT, 0, item_height as LPARAM);
        MoveWindow(
            edit,
            px(8, dpi),
            px(8, dpi),
            px(WIDTH - 16, dpi),
            px(30, dpi),
            1,
        );
        MoveWindow(
            list,
            px(8, dpi),
            px(40, dpi),
            px(WIDTH - 16, dpi),
            item_height * 8,
            1,
        );
        DeleteObject(old_font as _);
        DeleteObject(old_shortcut_font as _);
    }
    unsafe { InvalidateRect(hwnd, null(), 1) };
}

unsafe fn show_launcher(hwnd: HWND) {
    let mut cursor = unsafe { std::mem::zeroed() };
    unsafe { GetCursorPos(&mut cursor) };
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } < 0 {
        let Some(current_dpi) = (unsafe { with_app(hwnd, |state| state.dpi) }) else {
            return;
        };
        dpi_x = current_dpi;
    }
    unsafe { apply_dpi(hwnd, dpi_x) };
    let dpi = dpi_x;
    let width = px(WIDTH, dpi);
    let height = px(HEIGHT, dpi);
    let mut monitor_info: MONITORINFO = unsafe { std::mem::zeroed() };
    monitor_info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    unsafe { GetMonitorInfoW(monitor, &mut monitor_info) };
    let work = monitor_info.rcWork;
    let x = work.left + (work.right - work.left - width) / 2;
    let y = work.top + (work.bottom - work.top - height) / 3;
    unsafe {
        SetWindowPos(hwnd, HWND_TOPMOST, x, y, width, height, SWP_SHOWWINDOW);
        SetForegroundWindow(hwnd)
    };
    let Some(edit) = (unsafe { with_app(hwnd, |state| state.edit) }) else {
        return;
    };
    unsafe {
        SetWindowTextW(edit, wide("").as_ptr());
        SetFocus(edit)
    };
    unsafe { refresh(hwnd) };
}

unsafe fn show_manual(hwnd: HWND) {
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        unsafe { show_launcher(hwnd) };
    }
    unsafe { set_launcher_query(hwnd, "?") };
}

unsafe fn show_settings(hwnd: HWND) {
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        unsafe { show_launcher(hwnd) };
    }
    unsafe { set_launcher_query(hwnd, "settings") };
}

unsafe fn show_tray_menu(hwnd: HWND) {
    let checked = if unsafe { with_app(hwnd, |state| state.light_theme) }.unwrap_or(false) {
        MF_CHECKED
    } else {
        0
    };
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    unsafe {
        AppendMenuW(menu, MF_STRING, MENU_SHOW, wide("显示启动器").as_ptr());
        AppendMenuW(menu, MF_STRING, MENU_HELP, wide("使用说明\tF1").as_ptr());
        AppendMenuW(menu, MF_STRING, MENU_SETTINGS, wide("设置中心").as_ptr());
        AppendMenuW(menu, MF_STRING, MENU_REFRESH, wide("刷新索引").as_ptr());
        AppendMenuW(
            menu,
            MF_STRING | checked,
            MENU_LIGHT,
            wide("亮色皮肤").as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, MENU_EXIT, wide("退出").as_ptr());
        let mut point = std::mem::zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        TrackPopupMenu(menu, TPM_RIGHTBUTTON, point.x, point.y, 0, hwnd, null());
        DestroyMenu(menu);
    }
}

unsafe fn toggle_theme(hwnd: HWND) {
    let Some(light_theme) = (unsafe { with_app(hwnd, |state| !state.light_theme) }) else {
        return;
    };
    let colors = theme(light_theme);
    let background = unsafe { CreateSolidBrush(colors.bg) };
    let panel = unsafe { CreateSolidBrush(colors.bg) };
    if background.is_null() || panel.is_null() {
        unsafe {
            if !background.is_null() {
                DeleteObject(background as _);
            }
            if !panel.is_null() {
                DeleteObject(panel as _);
            }
            show_action_error(hwnd, "无法创建主题绘制资源。")
        };
        return;
    }
    let swapped = unsafe {
        with_app_mut(hwnd, |state| {
            let old = (state.background, state.panel, state.edit, state.list);
            state.light_theme = light_theme;
            state.background = background;
            state.panel = panel;
            old
        })
    };
    let Some((old_background, old_panel, edit, list)) = swapped else {
        unsafe {
            DeleteObject(background as _);
            DeleteObject(panel as _);
        }
        return;
    };
    unsafe {
        DeleteObject(old_background as _);
        DeleteObject(old_panel as _)
    };
    if !save_theme(light_theme) {
        unsafe {
            show_action_error(hwnd, "主题已切换，但无法保存；下次启动可能恢复原设置。")
        };
    }
    unsafe {
        InvalidateRect(hwnd, null(), 1);
        InvalidateRect(edit, null(), 1);
        InvalidateRect(list, null(), 1);
        UpdateWindow(hwnd);
    }
}

fn hotkey_binding(hotkey: HotkeyChoice) -> (u32, u32) {
    match hotkey {
        HotkeyChoice::AltSpace => (MOD_ALT, 0x20),
        HotkeyChoice::ControlSpace => (MOD_CONTROL, 0x20),
        HotkeyChoice::AltQ => (MOD_ALT, 0x51),
        HotkeyChoice::ControlAltSpace => (MOD_CONTROL | MOD_ALT, 0x20),
    }
}

fn hotkey_candidates(start: HotkeyChoice) -> [HotkeyChoice; 4] {
    let second = start.next();
    let third = second.next();
    [start, second, third, third.next()]
}

unsafe fn register_launcher_hotkey(hwnd: HWND, hotkey: HotkeyChoice) -> bool {
    let (modifiers, key) = hotkey_binding(hotkey);
    unsafe { RegisterHotKey(hwnd, HOTKEY_ID, modifiers | MOD_NOREPEAT, key) != 0 }
}

unsafe fn register_available_hotkey(hwnd: HWND, start: HotkeyChoice) -> Option<HotkeyChoice> {
    hotkey_candidates(start)
        .into_iter()
        .find(|hotkey| unsafe { register_launcher_hotkey(hwnd, *hotkey) })
}

unsafe fn apply_setting_change(hwnd: HWND, change: SettingChange) {
    match change {
        SettingChange::CycleHotkey => {
            let Some((current, was_registered)) = (unsafe {
                with_app(hwnd, |state| {
                    (state.settings.hotkey, state.hotkey_registered)
                })
            }) else {
                return;
            };
            unsafe { UnregisterHotKey(hwnd, HOTKEY_ID) };
            match unsafe { register_available_hotkey(hwnd, current.next()) } {
                Some(next) => {
                    let updated = unsafe {
                        with_app_mut(hwnd, |state| {
                            state.settings.hotkey = next;
                            state.hotkey_registered = true;
                        })
                    };
                    if updated.is_none() {
                        unsafe { UnregisterHotKey(hwnd, HOTKEY_ID) };
                        return;
                    }
                    let tray = unsafe { tray_data(hwnd) };
                    unsafe { Shell_NotifyIconW(NIM_MODIFY, &tray) };
                    if next == current && was_registered {
                        unsafe {
                            show_action_error(
                                hwnd,
                                &format!("其他可选快捷键均被占用，继续使用 {}。", current.label()),
                            )
                        };
                    }
                }
                None => {
                    if unsafe { with_app_mut(hwnd, |state| state.hotkey_registered = false) }
                        .is_none()
                    {
                        return;
                    }
                    let tray = unsafe { tray_data(hwnd) };
                    unsafe { Shell_NotifyIconW(NIM_MODIFY, &tray) };
                    unsafe {
                        show_action_error(
                            hwnd,
                            "所有可选全局快捷键均被占用。QuickSpark 会继续在托盘运行，请释放冲突快捷键后重试。",
                        )
                    };
                }
            }
        }
        SettingChange::ToggleAutoStart => {
            let Some(enabled) = (unsafe { with_app(hwnd, |state| !state.settings.auto_start) })
            else {
                return;
            };
            if set_autostart(enabled) {
                if unsafe { with_app_mut(hwnd, |state| state.settings.auto_start = enabled) }
                    .is_none()
                {
                    let _ = set_autostart(!enabled);
                    return;
                }
            } else {
                unsafe { show_action_error(hwnd, "无法更改开机自动启动设置。") };
            }
        }
        SettingChange::ToggleTheme => unsafe { toggle_theme(hwnd) },
        SettingChange::CycleResultLimit => {
            if unsafe {
                with_app_mut(hwnd, |state| {
                    state.settings.result_limit = match state.settings.result_limit {
                        8 => 12,
                        12 => 20,
                        _ => 8,
                    };
                })
            }
            .is_none()
            {
                return;
            }
        }
        SettingChange::CycleWebEngine => {
            if unsafe {
                with_app_mut(hwnd, |state| {
                    state.settings.web_engine = state.settings.web_engine.next();
                })
            }
            .is_none()
            {
                return;
            }
        }
    }
    let Some(settings) = (unsafe { with_app(hwnd, |state| state.settings.clone()) }) else {
        return;
    };
    if !save_settings(&settings) {
        unsafe { show_action_error(hwnd, "设置已应用，但无法保存到磁盘。") };
    }
    unsafe { refresh(hwnd) };
}

unsafe fn execute_system_command(hwnd: HWND, command: SystemCommand) -> bool {
    let confirmation = match command {
        SystemCommand::Lock => None,
        SystemCommand::Sleep => Some("确定要让电脑进入睡眠吗？"),
        SystemCommand::SignOut => Some("确定要注销当前用户吗？请先保存正在进行的工作。"),
        SystemCommand::Shutdown => Some("确定要关闭电脑吗？请先保存正在进行的工作。"),
        SystemCommand::Restart => Some("确定要重新启动电脑吗？请先保存正在进行的工作。"),
    };
    if let Some(message) = confirmation
        && unsafe {
            MessageBoxW(
                hwnd,
                wide(message).as_ptr(),
                wide("QuickSpark 系统命令").as_ptr(),
                MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
            )
        } != IDYES
    {
        return false;
    }

    let succeeded = match command {
        SystemCommand::Lock => unsafe { LockWorkStation() != 0 },
        SystemCommand::Sleep => unsafe { SetSuspendState(false, false, false) },
        SystemCommand::SignOut => windows_program(r"System32\shutdown.exe")
            .is_some_and(|target| unsafe { shell_execute(hwnd, "open", &target, Some("/l")) }),
        SystemCommand::Shutdown => windows_program(r"System32\shutdown.exe")
            .is_some_and(|target| unsafe { shell_execute(hwnd, "open", &target, Some("/s /t 0")) }),
        SystemCommand::Restart => windows_program(r"System32\shutdown.exe")
            .is_some_and(|target| unsafe { shell_execute(hwnd, "open", &target, Some("/r /t 0")) }),
    };
    if !succeeded {
        unsafe { show_action_error(hwnd, "系统命令执行失败。") };
    }
    succeeded
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if TASKBAR_CREATED
        .get()
        .is_some_and(|taskbar_created| *taskbar_created == msg)
        && unsafe { has_app(hwnd) }
    {
        let tray = unsafe { tray_data(hwnd) };
        unsafe { Shell_NotifyIconW(NIM_ADD, &tray) };
        return 0;
    }
    match msg {
        WM_CREATE => {
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            let light_theme = load_light_theme();
            let mut settings = load_settings();
            let colors = theme(light_theme);
            let face = wide("Microsoft YaHei UI");
            let font = unsafe {
                CreateFontW(
                    px(-19, dpi),
                    0,
                    0,
                    0,
                    FW_NORMAL as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    CLEARTYPE_QUALITY as u32,
                    FF_DONTCARE as u32,
                    face.as_ptr(),
                )
            };
            let shortcut_font = unsafe {
                CreateFontW(
                    px(-18, dpi),
                    0,
                    0,
                    0,
                    FW_NORMAL as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    CLEARTYPE_QUALITY as u32,
                    FF_DONTCARE as u32,
                    face.as_ptr(),
                )
            };
            let background = unsafe { CreateSolidBrush(colors.bg) };
            let panel = unsafe { CreateSolidBrush(colors.bg) };
            let item_height = px(32, dpi);
            let edit = unsafe {
                CreateWindowExW(
                    0,
                    wide("EDIT").as_ptr(),
                    null(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                    px(8, dpi),
                    px(8, dpi),
                    px(WIDTH - 16, dpi),
                    px(30, dpi),
                    hwnd,
                    EDIT_ID as _,
                    null_mut(),
                    null(),
                )
            };
            let list = unsafe {
                CreateWindowExW(
                    0,
                    wide("LISTBOX").as_ptr(),
                    null(),
                    WS_CHILD
                        | WS_VISIBLE
                        | LBS_NOTIFY as u32
                        | LBS_NOINTEGRALHEIGHT as u32
                        | LBS_OWNERDRAWFIXED as u32,
                    px(8, dpi),
                    px(40, dpi),
                    px(WIDTH - 16, dpi),
                    item_height * VISIBLE_RESULT_COUNT as i32,
                    hwnd,
                    LIST_ID as _,
                    null_mut(),
                    null(),
                )
            };
            if font.is_null()
                || shortcut_font.is_null()
                || background.is_null()
                || panel.is_null()
                || edit.is_null()
                || list.is_null()
            {
                unsafe {
                    if !font.is_null() {
                        DeleteObject(font as _);
                    }
                    if !shortcut_font.is_null() {
                        DeleteObject(shortcut_font as _);
                    }
                    if !background.is_null() {
                        DeleteObject(background as _);
                    }
                    if !panel.is_null() {
                        DeleteObject(panel as _);
                    }
                    MessageBoxW(
                        hwnd,
                        wide("无法创建主窗口所需的控件或绘制资源，QuickSpark 将退出。").as_ptr(),
                        wide("QuickSpark").as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                }
                // Returning -1 aborts parent creation; Windows destroys any child
                // controls that were successfully created before the failure.
                return -1;
            }
            unsafe {
                SendMessageW(edit, WM_SETFONT, font as WPARAM, 1);
                SendMessageW(list, WM_SETFONT, font as WPARAM, 1);
                // Tight pure-text layout: 8px origin + 8px margin = 16px.
                let left_margin = px(8, dpi) as u32;
                let right_margin = px(8, dpi) as u32;
                SendMessageW(
                    edit,
                    EM_SETMARGINS,
                    (EC_LEFTMARGIN | EC_RIGHTMARGIN) as WPARAM,
                    (left_margin | (right_margin << 16)) as LPARAM,
                );
                SendMessageW(
                    edit,
                    EM_SETCUEBANNER,
                    1,
                    wide("搜索应用，输入 ? 查看说明").as_ptr() as LPARAM,
                );
                SendMessageW(list, LB_SETITEMHEIGHT, 0, item_height as LPARAM);
            };
            let preferred_hotkey = settings.hotkey;
            let registered_hotkey = unsafe { register_available_hotkey(hwnd, preferred_hotkey) };
            let hotkey_registered = registered_hotkey.is_some();
            let hotkey_notice = match registered_hotkey {
                Some(active) if active != preferred_hotkey => {
                    settings.hotkey = active;
                    let saved = save_settings(&settings);
                    Some(format!(
                        "{} 已被其他程序占用，已自动改用 {}。{}",
                        preferred_hotkey.label(),
                        active.label(),
                        if saved {
                            "新快捷键已保存。"
                        } else {
                            "本次启动可用，但无法保存设置。"
                        }
                    ))
                }
                None => Some(
                    "所有可选全局快捷键均被占用。QuickSpark 已继续启动，请从系统托盘打开，并在释放冲突快捷键后进入设置中心重试。"
                        .to_owned(),
                ),
                _ => None,
            };
            let (index_tx, index_rx) = mpsc::channel();
            let mut providers = ProviderRegistry::new();
            let ai_service = AiService::new(hwnd as usize, AI_READY);
            providers.register(HelpProvider::new());
            providers.register(SettingsProvider::new());
            providers.register(CommandProvider::new());
            providers.register(SystemProvider::new());
            providers.register(WebSearchProvider::new());
            providers.register(CalculatorProvider::new());
            providers.register(ConverterProvider::new());
            providers.register(WindowSwitcherProvider::new());
            providers.register(FileSearchProvider::new(hwnd as usize, FILE_SEARCH_READY));
            providers.register(ai_service.provider());
            providers.register(ApplicationProvider::empty());
            let state = Box::new(RefCell::new(App {
                edit,
                list,
                results: Vec::new(),
                providers,
                font,
                shortcut_font,
                background,
                panel,
                dpi,
                light_theme,
                usage: load_usage(),
                pinned: load_pinned(),
                settings,
                hotkey_registered,
                ai_service,
                ai_window: null_mut(),
                ai_generation: 0,
                indexing: false,
                index_tx,
                index_rx,
            }));
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize) };
            let tray = unsafe { tray_data(hwnd) };
            let tray_added = unsafe { Shell_NotifyIconW(NIM_ADD, &tray) } != 0;
            if !tray_added {
                let access_message = if hotkey_registered {
                    let hotkey = unsafe { with_app(hwnd, |state| state.settings.hotkey.label()) }
                        .unwrap_or("已注册快捷键");
                    format!("无法创建托盘图标，但 {} 仍可使用。", hotkey)
                } else {
                    "无法创建托盘图标，且全局快捷键不可用；主窗口将直接打开。".to_owned()
                };
                unsafe {
                    MessageBoxW(
                        hwnd,
                        wide(&access_message).as_ptr(),
                        wide("QuickSpark").as_ptr(),
                        MB_OK | MB_ICONWARNING,
                    )
                };
            }
            if let Some(message) = hotkey_notice {
                unsafe {
                    MessageBoxW(
                        hwnd,
                        wide(&message).as_ptr(),
                        wide("QuickSpark 快捷键").as_ptr(),
                        MB_OK | MB_ICONWARNING,
                    )
                };
            }
            unsafe { start_index_refresh(hwnd) };
            unsafe { refresh(hwnd) };
            if !tray_added && !hotkey_registered {
                unsafe {
                    ShowWindow(hwnd, SW_SHOW);
                    SetForegroundWindow(hwnd);
                    SetFocus(edit);
                }
            }
            0
        }
        INDEX_READY => {
            let Some(result) = (unsafe {
                with_app_mut(hwnd, |state| {
                    state.indexing = false;
                    state.index_rx.try_recv()
                })
            }) else {
                return 0;
            };
            match result {
                Ok(Ok(provider)) => {
                    let _ =
                        unsafe { with_app_mut(hwnd, |state| state.providers.register(provider)) };
                }
                Ok(Err(_)) => unsafe {
                    MessageBoxW(
                        hwnd,
                        wide("刷新应用索引时发生内部错误。现有搜索结果将继续保留，您可以稍后从托盘菜单重试。").as_ptr(),
                        wide("QuickSpark").as_ptr(),
                        MB_OK | MB_ICONWARNING,
                    );
                },
                Err(_) => {}
            }
            unsafe { refresh(hwnd) };
            0
        }
        FILE_SEARCH_READY => {
            unsafe { refresh(hwnd) };
            0
        }
        AI_READY => {
            let Some((events, active_generation, answer_window)) = (unsafe {
                with_app(hwnd, |state| {
                    (
                        state.ai_service.drain_events(),
                        state.ai_generation,
                        state.ai_window,
                    )
                })
            }) else {
                return 0;
            };
            if events.is_empty() {
                return 0;
            }
            let mut reasoning_delta = String::new();
            let mut answer_delta = String::new();
            let mut terminal = None;
            for event in events {
                if event.generation() != active_generation {
                    continue;
                }
                match event {
                    AiEvent::ReasoningDelta { text, .. } => reasoning_delta.push_str(&text),
                    AiEvent::AnswerDelta { text, .. } => answer_delta.push_str(&text),
                    completed @ (AiEvent::Finished { .. } | AiEvent::Failed { .. }) => {
                        terminal = Some(completed);
                    }
                }
            }
            if !answer_window.is_null() {
                if !reasoning_delta.is_empty() {
                    append_ai_reasoning(answer_window, &reasoning_delta);
                }
                if !answer_delta.is_empty() {
                    append_ai_answer(answer_window, &answer_delta);
                }
                match terminal.as_ref() {
                    Some(AiEvent::Finished { finish_reason, .. }) => {
                        finish_ai_answer(answer_window, finish_reason.as_deref())
                    }
                    Some(AiEvent::Failed { error, .. }) => fail_ai_answer(answer_window, error),
                    _ => {}
                }
            }
            if terminal.is_some() {
                unsafe { refresh(hwnd) };
            }
            0
        }
        AI_CANCEL => {
            let generation = w as u64;
            let answer_window = l as HWND;
            let answer_window_destroyed = unsafe { IsWindow(answer_window) } == 0;
            let _ = unsafe {
                with_app_mut(hwnd, |state| {
                    if state.ai_generation == generation {
                        state.ai_service.cancel();
                        if state.ai_window == answer_window && answer_window_destroyed {
                            state.ai_window = null_mut();
                            state.ai_generation = 0;
                        }
                    }
                })
            };
            cancel_ai_answer(answer_window);
            unsafe { refresh(hwnd) };
            0
        }
        AI_WINDOW_CLOSED => {
            let generation = w as u64;
            let answer_window = l as HWND;
            let _ = unsafe {
                with_app_mut(hwnd, |state| {
                    if state.ai_generation == generation && state.ai_window == answer_window {
                        state.ai_window = null_mut();
                    }
                })
            };
            0
        }
        SHOW_EXISTING => {
            unsafe { show_launcher(hwnd) };
            0
        }
        WM_DPICHANGED => {
            let dpi = (w & 0xffff) as u32;
            let rect = unsafe { &*(l as *const RECT) };
            unsafe {
                apply_dpi(hwnd, dpi);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            };
            0
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            let Some((light_theme, panel)) =
                (unsafe { with_app(hwnd, |state| (state.light_theme, state.panel)) })
            else {
                return unsafe { DefWindowProcW(hwnd, msg, w, l) };
            };
            let colors = theme(light_theme);
            unsafe {
                SetTextColor(w as _, colors.text);
                SetBkColor(w as _, colors.bg);
            }
            panel as LRESULT
        }
        WM_ERASEBKGND => {
            let Some(background) = (unsafe { with_app(hwnd, |state| state.background) }) else {
                return unsafe { DefWindowProcW(hwnd, msg, w, l) };
            };
            let mut rect = unsafe { std::mem::zeroed() };
            unsafe {
                GetClientRect(hwnd, &mut rect);
                windows_sys::Win32::Graphics::Gdi::FillRect(w as _, &rect, background)
            };
            1
        }
        WM_PAINT => {
            let Some((light_theme, dpi)) =
                (unsafe { with_app(hwnd, |state| (state.light_theme, state.dpi)) })
            else {
                return unsafe { DefWindowProcW(hwnd, msg, w, l) };
            };
            let colors = theme(light_theme);
            let mut paint: PAINTSTRUCT = unsafe { std::mem::zeroed() };
            let dc = unsafe { BeginPaint(hwnd, &mut paint) };
            let pen = unsafe { CreatePen(PS_SOLID, 1, colors.divider) };
            let old_pen = unsafe { SelectObject(dc, pen as _) };
            let y = px(39, dpi);
            unsafe {
                MoveToEx(dc, px(8, dpi), y, null_mut());
                LineTo(dc, px(WIDTH - 8, dpi), y);
                SelectObject(dc, old_pen);
                DeleteObject(pen as _);
                EndPaint(hwnd, &paint)
            };
            0
        }
        WM_DRAWITEM => {
            let draw = unsafe { &*(l as *const DRAWITEMSTRUCT) };
            if draw.CtlID == LIST_ID as u32 && draw.itemID != u32::MAX {
                let Some((light_theme, display_name, panel, dpi, font, shortcut_font)) = (unsafe {
                    with_app(hwnd, |state| {
                        (
                            state.light_theme,
                            state
                                .results
                                .get(draw.itemID as usize)
                                .map(|result| result.title.clone()),
                            state.panel,
                            state.dpi,
                            state.font,
                            state.shortcut_font,
                        )
                    })
                }) else {
                    return unsafe { DefWindowProcW(hwnd, msg, w, l) };
                };
                let colors = theme(light_theme);
                if let Some(display_name) = display_name {
                    let selected = draw.itemState & ODS_SELECTED != 0;
                    unsafe { FillRect(draw.hDC, &draw.rcItem, panel) };
                    if selected {
                        let brush = unsafe { CreateSolidBrush(colors.selected) };
                        let old_brush = unsafe { SelectObject(draw.hDC, brush as _) };
                        let old_pen = unsafe { SelectObject(draw.hDC, GetStockObject(NULL_PEN)) };
                        let inset = px(3, dpi);
                        unsafe {
                            RoundRect(
                                draw.hDC,
                                draw.rcItem.left + inset,
                                draw.rcItem.top + px(1, dpi),
                                draw.rcItem.right - inset,
                                draw.rcItem.bottom - px(1, dpi),
                                px(8, dpi),
                                px(8, dpi),
                            )
                        };
                        unsafe {
                            SelectObject(draw.hDC, old_pen);
                            SelectObject(draw.hDC, old_brush);
                            DeleteObject(brush as _)
                        };
                    }

                    let padding = px(8, dpi);
                    let shortcut_width = px(82, dpi);
                    let mut name_rect = draw.rcItem;
                    name_rect.left += padding;
                    name_rect.right -= shortcut_width;
                    let mut shortcut_rect = draw.rcItem;
                    shortcut_rect.left = shortcut_rect.right - shortcut_width;
                    shortcut_rect.right -= padding;
                    let name = wide(&display_name);
                    let shortcut = (draw.itemID < VISIBLE_RESULT_COUNT as u32)
                        .then(|| wide(&format!("Alt+{}", draw.itemID + 1)));
                    unsafe {
                        SelectObject(draw.hDC, font as _);
                        SetBkMode(draw.hDC, TRANSPARENT as i32);
                        SetTextColor(draw.hDC, if selected { colors.text } else { colors.dim });
                        DrawTextW(
                            draw.hDC,
                            name.as_ptr(),
                            -1,
                            &mut name_rect,
                            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                        );
                        if let Some(shortcut) = shortcut {
                            SelectObject(draw.hDC, shortcut_font as _);
                            SetTextColor(draw.hDC, colors.muted);
                            DrawTextW(
                                draw.hDC,
                                shortcut.as_ptr(),
                                -1,
                                &mut shortcut_rect,
                                DT_RIGHT | DT_SINGLELINE | DT_VCENTER,
                            );
                        }
                    }
                }
            }
            1
        }
        WM_HOTKEY => {
            if unsafe { IsWindowVisible(hwnd) } != 0 {
                unsafe { ShowWindow(hwnd, SW_HIDE) };
            } else {
                unsafe { show_launcher(hwnd) };
            }
            0
        }
        TRAY_MESSAGE => {
            match l as u32 {
                WM_LBUTTONDBLCLK => unsafe { show_launcher(hwnd) },
                WM_RBUTTONUP | WM_CONTEXTMENU => unsafe { show_tray_menu(hwnd) },
                _ => {}
            }
            0
        }
        WM_COMMAND if w == MENU_SHOW => {
            unsafe { show_launcher(hwnd) };
            0
        }
        WM_COMMAND if w == MENU_HELP => {
            unsafe { show_manual(hwnd) };
            0
        }
        WM_COMMAND if w == MENU_SETTINGS => {
            unsafe { show_settings(hwnd) };
            0
        }
        WM_COMMAND if w == MENU_REFRESH => {
            unsafe { start_index_refresh(hwnd) };
            0
        }
        WM_COMMAND if w == MENU_LIGHT => {
            unsafe { toggle_theme(hwnd) };
            0
        }
        WM_COMMAND if w == MENU_EXIT => {
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_CLOSE => {
            unsafe { ShowWindow(hwnd, SW_HIDE) };
            0
        }
        WM_COMMAND => {
            let id = (w & 0xffff) as i32;
            let code = ((w >> 16) & 0xffff) as u16;
            if id == EDIT_ID && code == EN_CHANGE as u16 {
                unsafe { refresh(hwnd) };
            }
            if id == LIST_ID && code == LBN_DBLCLK as u16 {
                unsafe { launch_selected(hwnd) };
            }
            0
        }
        WM_KEYDOWN => {
            match w as u16 {
                VK_ESCAPE => unsafe {
                    ShowWindow(hwnd, SW_HIDE);
                },
                VK_RETURN => unsafe { launch_selected(hwnd) },
                VK_UP => unsafe { move_selection(hwnd, -1) },
                VK_DOWN => unsafe { move_selection(hwnd, 1) },
                _ => return unsafe { DefWindowProcW(hwnd, msg, w, l) },
            }
            0
        }
        WM_ACTIVATE => {
            if (w & 0xffff) as u32 == WA_INACTIVE {
                unsafe { ShowWindow(hwnd, SW_HIDE) };
            }
            0
        }
        WM_DESTROY => {
            unsafe { UnregisterHotKey(hwnd, HOTKEY_ID) };
            let tray = unsafe { tray_data(hwnd) };
            unsafe { Shell_NotifyIconW(NIM_DELETE, &tray) };
            let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AppCell;
            if !ptr.is_null() {
                let can_drop = unsafe { (&*ptr).try_borrow_mut().is_ok() };
                // Clear the window-owned pointer before dropping App. A synchronous
                // message during teardown must never observe already-freed state.
                unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
                if can_drop {
                    let cell = unsafe { Box::from_raw(ptr) };
                    let state = RefCell::into_inner(*cell);
                    state.ai_service.cancel();
                    unsafe {
                        DeleteObject(state.font as _);
                        DeleteObject(state.shortcut_font as _);
                        DeleteObject(state.background as _);
                        DeleteObject(state.panel as _);
                    }
                }
                // If destruction was synchronously re-entered while state was borrowed,
                // leave the allocation alive until process exit rather than invalidating
                // the active reference. Windows reclaims the process resources immediately.
            }
            unsafe { PostQuitMessage(0) };
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}

fn main() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let taskbar_message = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        if taskbar_message != 0 {
            let _ = TASKBAR_CREATED.set(taskbar_message);
        }
        migrate_legacy_data();
        let mutex_name = wide("Local\\QuickSpark.SingleInstance");
        let instance_mutex = CreateMutexW(null(), 0, mutex_name.as_ptr());
        if instance_mutex.is_null() {
            return;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let existing = FindWindowW(wide("QuickSparkWindow").as_ptr(), null());
            if !existing.is_null() {
                PostMessageW(existing, SHOW_EXISTING, 0, 0);
            }
            CloseHandle(instance_mutex);
            return;
        }
        let dpi = GetDpiForSystem();
        let instance: HINSTANCE = GetModuleHandleW(null());
        let class = wide("QuickSparkWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hIcon: application_icon(),
            hbrBackground: (COLOR_WINDOW + 1) as _,
            ..std::mem::zeroed()
        };
        if RegisterClassW(&wc) == 0 {
            CloseHandle(instance_mutex);
            return;
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            wide("QuickSpark").as_ptr(),
            WS_POPUP,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            px(WIDTH, dpi),
            px(HEIGHT, dpi),
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            CloseHandle(instance_mutex);
            return;
        }
        let corner = DWMWCP_ROUND;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &corner as *const _ as _,
            std::mem::size_of_val(&corner) as u32,
        );
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if let Some(position) = alt_shortcut_index(msg.message, msg.wParam) {
                launch_result(hwnd, position);
                continue;
            }
            let launcher_control = with_app(hwnd, |state| {
                msg.hwnd == state.edit || msg.hwnd == state.list
            })
            .unwrap_or(false);
            if msg.message == WM_KEYDOWN
                && msg.wParam as u16 == VK_F1
                && (msg.hwnd == hwnd || launcher_control)
            {
                show_manual(hwnd);
                continue;
            }
            if msg.message == WM_MOUSEWHEEL && (msg.hwnd == hwnd || launcher_control) {
                let delta = wheel_selection_delta(msg.wParam);
                if delta != 0 {
                    move_selection(hwnd, delta);
                }
                continue;
            }
            if msg.message == WM_KEYDOWN && launcher_control {
                if handle_action_shortcut(hwnd, msg.wParam as u16) {
                    continue;
                }
                match msg.wParam as u16 {
                    VK_ESCAPE => {
                        ShowWindow(hwnd, SW_HIDE);
                        continue;
                    }
                    VK_RETURN => {
                        launch_selected(hwnd);
                        continue;
                    }
                    VK_UP => {
                        move_selection(hwnd, -1);
                        continue;
                    }
                    VK_DOWN => {
                        move_selection(hwnd, 1);
                        continue;
                    }
                    _ => {}
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        CloseHandle(instance_mutex);
    }
}

#[cfg(test)]
mod tests;
