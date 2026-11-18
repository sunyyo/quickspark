use std::{os::windows::ffi::OsStrExt, ptr::null};

use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, ERROR_NOT_FOUND, GetLastError, HWND, NO_ERROR},
    Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CREDUI_FLAGS_ALWAYS_SHOW_UI,
        CREDUI_FLAGS_DO_NOT_PERSIST, CREDUI_FLAGS_GENERIC_CREDENTIALS, CREDUI_FLAGS_KEEP_USERNAME,
        CREDUI_INFOW, CredDeleteW, CredFree, CredReadW, CredUIPromptForCredentialsW, CredWriteW,
    },
    UI::WindowsAndMessaging::{
        IDCANCEL, IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_YESNOCANCEL, MessageBoxW,
    },
};

use super::{ParsedUrl, wide};

const LEGACY_AI_TARGET: &str = "QuickSpark/AI/APIKey";
const TAVILY_TARGET: &str = "QuickSpark/Tavily/APIKey";

pub(crate) enum ApiKeyPromptResult {
    Saved,
    Cleared,
    Cancelled,
}

fn ai_target(origin: &str) -> String {
    format!("QuickSpark/AI/APIKey/{origin}")
}

pub(crate) fn ai_api_key_exists(api_url: &str) -> bool {
    load_ai_api_key(api_url).ok().flatten().is_some()
}

pub(crate) fn load_ai_api_key(api_url: &str) -> Result<Option<String>, String> {
    let endpoint = ParsedUrl::parse(api_url)?;
    load_api_key(&ai_target(&endpoint.origin()))
}

pub(crate) fn tavily_api_key_exists() -> bool {
    load_tavily_api_key().ok().flatten().is_some()
}

pub(crate) fn load_tavily_api_key() -> Result<Option<String>, String> {
    load_api_key(TAVILY_TARGET)
}

fn load_api_key(target_name: &str) -> Result<Option<String>, String> {
    let target = wide(target_name);
    let mut credential = std::ptr::null_mut();
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) } == 0 {
        let error = unsafe { GetLastError() };
        return if error == ERROR_NOT_FOUND {
            Ok(None)
        } else {
            Err(format!("无法读取 API Key（Windows 错误 {error}）"))
        };
    }
    if credential.is_null() {
        return Err("凭据管理器返回了空凭据".to_owned());
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
    Ok(value.filter(|value| !value.is_empty()))
}

pub(crate) fn prompt_ai_api_key(parent: HWND, api_url: &str) -> Result<ApiKeyPromptResult, String> {
    let endpoint = ParsedUrl::parse(api_url)
        .map_err(|error| format!("请先在 ai settings 配置接口地址。{error}"))?;
    let origin = endpoint.origin();
    let target = ai_target(&origin);
    let existing = load_api_key(&target)?;
    if let Some(legacy) = load_api_key(LEGACY_AI_TARGET)?
        && existing.as_ref().is_none_or(|key| key == &legacy)
    {
        let message = format!(
            "将旧版 AI 密钥迁移到 {origin}？\n\n请选择“是”确认该接口属于原服务商。“否”可另外设置该接口的密钥，“取消”可返回。"
        );
        match unsafe {
            MessageBoxW(
                parent,
                wide(&message).as_ptr(),
                wide("确认密钥所属接口").as_ptr(),
                MB_YESNOCANCEL | MB_DEFBUTTON2 | MB_ICONQUESTION,
            )
        } {
            IDYES => {
                save_api_key(&target, "QuickSpark AI", &legacy)?;
                delete_api_key(LEGACY_AI_TARGET)
                    .map_err(|e| format!("已保存此接口的密钥，旧凭据清理失败。{e}"))?;
                return Ok(ApiKeyPromptResult::Saved);
            }
            IDCANCEL | 0 => return Ok(ApiKeyPromptResult::Cancelled),
            _ => {}
        }
    }
    prompt_api_key(
        parent,
        &target,
        "QuickSpark AI",
        "QuickSpark AI API Key",
        &format!("此密钥仅用于 {origin}。留空并确认可清除此接口的密钥。"),
        existing.as_deref(),
    )
}

pub(crate) fn prompt_tavily_api_key(parent: HWND) -> Result<ApiKeyPromptResult, String> {
    prompt_api_key(
        parent,
        TAVILY_TARGET,
        "QuickSpark Tavily",
        "QuickSpark Tavily API Key",
        "请输入 Tavily Search API Key。它会单独保存到 Windows 凭据管理器；留空并确认可清除已有 Key。",
        load_tavily_api_key()?.as_deref(),
    )
}

fn prompt_api_key(
    parent: HWND,
    target_name: &str,
    username_label: &str,
    caption_text: &str,
    message_text: &str,
    existing: Option<&str>,
) -> Result<ApiKeyPromptResult, String> {
    let message = wide(message_text);
    let caption = wide(caption_text);
    let target = wide(target_name);
    let mut username = vec![0u16; 64];
    copy_wide_to_buffer("API Key", &mut username);
    let mut password = vec![0u16; 2_048];
    if let Some(existing) = existing {
        copy_wide_to_buffer(existing, &mut password);
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
        delete_api_key(target_name)?;
        return Ok(ApiKeyPromptResult::Cleared);
    }
    if api_key.contains(['\r', '\n']) {
        return Err("API Key 不能包含换行符".to_owned());
    }
    save_api_key(target_name, username_label, api_key)?;
    Ok(ApiKeyPromptResult::Saved)
}

fn delete_api_key(target_name: &str) -> Result<(), String> {
    let target = wide(target_name);
    let error = if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        unsafe { GetLastError() }
    } else {
        NO_ERROR
    };
    deletion_result(error)
}

fn deletion_result(error: u32) -> Result<(), String> {
    match error {
        NO_ERROR | ERROR_NOT_FOUND => Ok(()),
        _ => Err(format!("无法清除 API Key（Windows 错误 {error}）")),
    }
}

fn save_api_key(target_name: &str, username_label: &str, api_key: &str) -> Result<(), String> {
    let mut target = wide(target_name);
    let mut username = wide(username_label);
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

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;

    #[test]
    fn keys_follow_the_endpoint_origin() {
        let target = |url| ai_target(&ParsedUrl::parse(url).unwrap().origin());
        let original = target("https://Example.COM/v1/chat/completions");
        assert_eq!(original, target("https://example.com:443/another/path?q=1"));
        assert_ne!(
            original,
            target("https://other.example.com/v1/chat/completions")
        );
        assert_ne!(
            original,
            target("https://example.com:8443/v1/chat/completions")
        );
        assert_ne!(original, target("http://127.0.0.1:8080"));
        assert_ne!(
            target("http://localhost:8080"),
            target("https://localhost:8080")
        );
        assert_eq!(
            target("http://[::1]:8080"),
            target("http://[0:0:0:0:0:0:0:1]:8080/api")
        );
        assert_ne!(original, LEGACY_AI_TARGET);
        assert_ne!(original, TAVILY_TARGET);
    }

    #[test]
    fn deleting_an_absent_key_succeeds_and_other_errors_are_reported() {
        assert!(deletion_result(NO_ERROR).is_ok());
        assert!(deletion_result(ERROR_NOT_FOUND).is_ok());
        assert!(deletion_result(ERROR_ACCESS_DENIED).is_err());
    }
}
