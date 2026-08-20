use std::{env, path::PathBuf, ptr::null};

use windows_sys::Win32::{
    Foundation::HWND,
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
};

use super::wide;

pub(crate) unsafe fn shell_execute(
    owner: HWND,
    verb: &str,
    target: &str,
    parameters: Option<&str>,
) -> bool {
    let verb = wide(verb);
    let target = wide(target);
    let parameters = parameters.map(wide);
    let parameter_pointer = parameters.as_ref().map_or(null(), |value| value.as_ptr());
    unsafe {
        ShellExecuteW(
            owner,
            verb.as_ptr(),
            target.as_ptr(),
            parameter_pointer,
            null(),
            SW_SHOWNORMAL,
        ) as isize
            > 32
    }
}

pub(crate) fn windows_program(relative: &str) -> Option<String> {
    let root = PathBuf::from(env::var_os("SystemRoot")?);
    let path = root.is_absolute().then(|| root.join(relative))?;
    path.is_file().then(|| path.to_string_lossy().into_owned())
}
