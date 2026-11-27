use std::{
    env,
    ffi::OsString,
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    ptr::{null, null_mut},
};

use windows_sys::Win32::{
    Foundation::HWND,
    Globalization::lstrlenW,
    System::Com::CoTaskMemFree,
    UI::{
        Shell::{SHGetKnownFolderPath, ShellExecuteW},
        WindowsAndMessaging::SW_SHOWNORMAL,
    },
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

pub(crate) fn known_folder_path(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    let mut path = null_mut();
    let status = unsafe { SHGetKnownFolderPath(id, 0, null_mut(), &mut path) };
    let result = (status >= 0 && !path.is_null()).then(|| unsafe {
        let units = std::slice::from_raw_parts(path, lstrlenW(path) as usize);
        PathBuf::from(OsString::from_wide(units))
    });
    unsafe { CoTaskMemFree(path.cast()) };
    result
}

pub(crate) fn windows_program(relative: &str) -> Option<PathBuf> {
    let root = PathBuf::from(env::var_os("SystemRoot")?);
    let path = root.is_absolute().then(|| root.join(relative))?;
    path.is_file().then_some(path)
}
