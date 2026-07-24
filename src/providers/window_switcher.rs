use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute},
    UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongW, GetWindowTextLengthW,
        GetWindowTextW, IsWindowVisible, WS_EX_TOOLWINDOW,
    },
};

use crate::{
    model::{Action, Query, ResultItem},
    search::fuzzy_score,
};

use super::{Provider, ProviderContext, ProviderOutput};

const PROVIDER_ID: &str = "window-switcher";

pub(crate) struct WindowSwitcherProvider;

impl WindowSwitcherProvider {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Provider for WindowSwitcherProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, _context: &ProviderContext<'_>) -> ProviderOutput {
        let Some(terms) = window_query(query.normalized()) else {
            return ProviderOutput::empty();
        };
        let results = enumerate_windows()
            .into_iter()
            .enumerate()
            .filter_map(|(position, window)| {
                let score = if terms.is_empty() {
                    100_000 - position as i32
                } else {
                    fuzzy_score(&window.title.to_lowercase(), terms)? + 100_000
                };
                Some(ResultItem {
                    provider_id: PROVIDER_ID,
                    title: window.title.clone(),
                    score,
                    actions: vec![
                        Action::ActivateWindow {
                            handle: window.handle,
                        },
                        Action::CopyText { text: window.title },
                    ],
                })
            })
            .collect();
        ProviderOutput {
            results,
            exclusive: true,
        }
    }
}

fn window_query(input: &str) -> Option<&str> {
    if input == "win" || input == "window" {
        return Some("");
    }
    input
        .strip_prefix("win ")
        .or_else(|| input.strip_prefix("window "))
        .map(str::trim)
}

struct WindowEntry {
    handle: isize,
    title: String,
}

fn enumerate_windows() -> Vec<WindowEntry> {
    let mut windows = Vec::new();
    unsafe {
        EnumWindows(
            Some(collect_window),
            &mut windows as *mut Vec<WindowEntry> as LPARAM,
        );
    }
    windows
}

unsafe extern "system" fn collect_window(window: HWND, data: LPARAM) -> i32 {
    if unsafe { IsWindowVisible(window) } == 0
        || !unsafe { GetWindow(window, GW_OWNER) }.is_null()
        || unsafe { GetWindowLongW(window, GWL_EXSTYLE) } & WS_EX_TOOLWINDOW as i32 != 0
    {
        return 1;
    }

    let mut cloaked = 0u32;
    let cloaked_result = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut u32 as _,
            std::mem::size_of::<u32>() as u32,
        )
    };
    if cloaked_result >= 0 && cloaked != 0 {
        return 1;
    }

    let length = unsafe { GetWindowTextLengthW(window) };
    if !(1..=1_024).contains(&length) {
        return 1;
    }
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
    if copied <= 0 {
        return 1;
    }
    let title = String::from_utf16_lossy(&buffer[..copied as usize]);
    if title.trim().is_empty() || title == "QuickSpark" || title.starts_with("QuickSpark AI") {
        return 1;
    }

    let windows = unsafe { &mut *(data as *mut Vec<WindowEntry>) };
    windows.push(WindowEntry {
        handle: window as isize,
        title,
    });
    1
}

#[cfg(test)]
mod tests {
    use super::{enumerate_windows, window_query};

    #[test]
    fn recognizes_window_prefixes_without_stealing_regular_queries() {
        assert_eq!(window_query("win"), Some(""));
        assert_eq!(window_query("win chrome"), Some("chrome"));
        assert_eq!(window_query("window obsidian"), Some("obsidian"));
        assert_eq!(window_query("windows terminal"), None);
        assert_eq!(window_query("winner"), None);
    }

    #[test]
    fn desktop_window_enumeration_is_safe_when_no_matches_exist() {
        let _ = enumerate_windows();
    }
}
