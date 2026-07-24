use windows_sys::Win32::{Foundation::SYSTEMTIME, System::SystemInformation::GetLocalTime};

pub(crate) fn local_date() -> (i32, u32, u32) {
    let mut time: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        GetLocalTime(&mut time);
    }
    (time.wYear as i32, time.wMonth as u32, time.wDay as u32)
}
