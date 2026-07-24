use std::ptr::copy_nonoverlapping;

use windows_sys::Win32::{
    Foundation::{GlobalFree, HWND},
    System::{
        DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock},
        Ole::CF_UNICODETEXT,
    },
};

struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

pub(crate) fn copy_text(owner: HWND, text: &str) -> bool {
    if unsafe { OpenClipboard(owner) } == 0 {
        return false;
    }
    let _guard = ClipboardGuard;
    if unsafe { EmptyClipboard() } == 0 {
        return false;
    }

    let encoded = text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let byte_count = encoded.len() * std::mem::size_of::<u16>();
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, byte_count) };
    if memory.is_null() {
        return false;
    }
    let destination = unsafe { GlobalLock(memory) } as *mut u16;
    if destination.is_null() {
        unsafe {
            GlobalFree(memory);
        }
        return false;
    }
    unsafe {
        copy_nonoverlapping(encoded.as_ptr(), destination, encoded.len());
        GlobalUnlock(memory);
    }

    let stored = unsafe { SetClipboardData(CF_UNICODETEXT as u32, memory as _) };
    if stored.is_null() {
        unsafe {
            GlobalFree(memory);
        }
        return false;
    }
    // Ownership of the global memory transfers to the system after SetClipboardData succeeds.
    true
}
