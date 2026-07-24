use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{IsIconic, IsWindow, SW_RESTORE, SetForegroundWindow, ShowWindow},
};

pub(crate) fn activate_window(handle: isize) -> bool {
    let window = handle as HWND;
    if window.is_null() || unsafe { IsWindow(window) } == 0 {
        return false;
    }
    if unsafe { IsIconic(window) } != 0 {
        unsafe {
            ShowWindow(window, SW_RESTORE);
        }
    }
    unsafe { SetForegroundWindow(window) != 0 }
}
