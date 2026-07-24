use std::{os::windows::ffi::OsStrExt, ptr::null};

use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, GetLastError, HWND, NO_ERROR},
    Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CREDUI_FLAGS_ALWAYS_SHOW_UI,
        CREDUI_FLAGS_DO_NOT_PERSIST, CREDUI_FLAGS_GENERIC_CREDENTIALS, CREDUI_FLAGS_KEEP_USERNAME,
        CREDUI_INFOW, CredDeleteW, CredFree, CredReadW, CredUIPromptForCredentialsW, CredWriteW,
    },
};

const TARGET: &str = "QuickSpark/AI/APIKey";

pub(crate) enum ApiKeyPromptResult {
    Saved,
    Cleared,
    Cancelled,
}

pub(crate) fn ai_api_key_exists() -> bool {
    load_ai_api_key().is_some()
}

pub(crate) fn load_ai_api_key() -> Option<String> {
    let target = wide(TARGET);
    let mut credential = std::ptr::null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0
        || credential.is_null()
    {
        return None;
    }
    let value = unsafe {
        let credential = &*credential;
        if credential.CredentialBlob.is_null() || credential.CredentialBlobSize == 0 {
            None
        } else {
            let bytes = std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            );
            String::from_utf8(bytes.to_vec()).ok()
        }
    };
    unsafe { CredFree(credential as _) };
    value.filter(|value| !value.is_empty())
}

pub(crate) fn prompt_ai_api_key(parent: HWND) -> Result<ApiKeyPromptResult, String> {
    let message = wide(
        "请输入 OpenAI 兼容接口的 API Key。它会保存到 Windows 凭据管理器；留空并确认可清除已有 Key。",
    );
    let caption = wide("QuickSpark AI API Key");
    let target = wide(TARGET);
    let mut username = vec![0u16; 64];
    copy_wide_to_buffer("API Key", &mut username);
    let mut password = vec![0u16; 2_048];
    if let Some(existing) = load_ai_api_key() {
        copy_wide_to_buffer(&existing, &mut password);
    }
    let info = CREDUI_INFOW {
        cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
        hwndParent: parent,
        pszMessageText: message.as_ptr(),
        pszCaptionText: caption.as_ptr(),
        hbmBanner: std::ptr::null_mut(),
    };
    let mut save = 0;
    let result = unsafe {
        CredUIPromptForCredentialsW(
            &info,
            target.as_ptr(),
            null(),
            0,
            username.as_mut_ptr(),
            username.len() as u32,
            password.as_mut_ptr(),
            password.len() as u32,
            &mut save,
            CREDUI_FLAGS_ALWAYS_SHOW_UI
                | CREDUI_FLAGS_DO_NOT_PERSIST
                | CREDUI_FLAGS_GENERIC_CREDENTIALS
                | CREDUI_FLAGS_KEEP_USERNAME,
        )
    };
    if result == ERROR_CANCELLED {
        password.fill(0);
        return Ok(ApiKeyPromptResult::Cancelled);
    }
    if result != NO_ERROR {
        password.fill(0);
        return Err(format!("Windows 凭据窗口返回错误 {result}"));
    }

    let length = password
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(password.len());
    let decoded = String::from_utf16(&password[..length]);
    password.fill(0);
    let api_key = decoded.map_err(|_| "API Key 包含无效字符".to_owned())?;
    let api_key = api_key.trim();
    if api_key.is_empty() {
        let deleted = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if deleted == 0 && ai_api_key_exists() {
            return Err(last_error("无法清除 API Key"));
        }
        return Ok(ApiKeyPromptResult::Cleared);
    }
    if api_key.contains(['\r', '\n']) {
        return Err("API Key 不能包含换行符".to_owned());
    }
    save_api_key(api_key)?;
    Ok(ApiKeyPromptResult::Saved)
}

fn save_api_key(api_key: &str) -> Result<(), String> {
    let mut target = wide(TARGET);
    let mut username = wide("QuickSpark AI");
    let mut bytes = api_key.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: bytes.len() as u32,
        CredentialBlob: bytes.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: username.as_mut_ptr(),
        ..Default::default()
    };
    let written = unsafe { CredWriteW(&credential, 0) };
    bytes.fill(0);
    if written == 0 {
        return Err(last_error("无法保存 API Key"));
    }
    Ok(())
}

fn copy_wide_to_buffer(value: &str, buffer: &mut [u16]) {
    if buffer.is_empty() {
        return;
    }
    let encoded = std::ffi::OsStr::new(value).encode_wide();
    let capacity = buffer.len() - 1;
    for (destination, source) in buffer.iter_mut().take(capacity).zip(encoded) {
        *destination = source;
    }
}

fn last_error(context: &str) -> String {
    format!("{context}（Windows 错误 {}）", unsafe {
        GetLastError()
    })
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(Some(0))
        .collect()
}
