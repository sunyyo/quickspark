use std::net::IpAddr;

use windows_sys::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_INTERNET_SCHEME_HTTP, WINHTTP_INTERNET_SCHEME_HTTPS, WinHttpCrackUrl,
};

use super::wide;

pub(crate) struct ParsedUrl {
    pub(crate) host: String,
    pub(crate) object: String,
    pub(crate) port: u16,
    pub(crate) secure: bool,
}

impl ParsedUrl {
    pub(crate) fn origin(&self) -> String {
        let scheme = if self.secure { "https" } else { "http" };
        let host = self.host.trim_start_matches('[').trim_end_matches(']');
        let host = match host.parse::<IpAddr>() {
            Ok(IpAddr::V6(address)) => format!("[{address}]"),
            _ => self.host.to_ascii_lowercase(),
        };
        format!("{scheme}://{host}:{}", self.port)
    }

    pub(crate) fn is_loopback(&self) -> bool {
        is_loopback_host(&self.host)
    }

    pub(crate) fn parse(url: &str) -> Result<Self, String> {
        let url = url.trim();
        if url.chars().any(char::is_control) {
            return Err("AI API 地址包含控制字符".to_owned());
        }
        let wide_url = wide(url);
        let mut components = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
            dwSchemeLength: u32::MAX,
            dwHostNameLength: u32::MAX,
            dwUserNameLength: u32::MAX,
            dwPasswordLength: u32::MAX,
            dwUrlPathLength: u32::MAX,
            dwExtraInfoLength: u32::MAX,
            ..Default::default()
        };
        if unsafe { WinHttpCrackUrl(wide_url.as_ptr(), 0, 0, &mut components) } == 0 {
            return Err(format!(
                "AI API 地址无效：{}",
                std::io::Error::last_os_error()
            ));
        }
        if components.dwUserNameLength != 0 || components.dwPasswordLength != 0 {
            return Err("请通过 ai key 设置凭据，接口地址中不能包含用户名或密码".to_owned());
        }
        let secure = match components.nScheme {
            WINHTTP_INTERNET_SCHEME_HTTPS => true,
            WINHTTP_INTERNET_SCHEME_HTTP => false,
            _ => return Err("AI API 地址只支持 http:// 或 https://".to_owned()),
        };
        let host = unsafe { wide_slice(components.lpszHostName, components.dwHostNameLength) };
        if host.is_empty() {
            return Err("AI API 地址缺少主机名".to_owned());
        }
        if !secure && !is_loopback_host(&host) {
            return Err(
                "为防止 API Key 和问题内容被明文传输，远程 AI 接口必须使用 https://；http:// 仅允许本机地址"
                    .to_owned(),
            );
        }
        let mut object = unsafe { wide_slice(components.lpszUrlPath, components.dwUrlPathLength) };
        object.push_str(&unsafe {
            wide_slice(components.lpszExtraInfo, components.dwExtraInfoLength)
        });
        if object.is_empty() {
            object.push('/');
        }
        Ok(Self {
            host,
            object,
            port: components.nPort,
            secure,
        })
    }
}

fn is_loopback_host(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.');
    normalized.eq_ignore_ascii_case("localhost")
        || normalized
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

unsafe fn wide_slice(pointer: *mut u16, length: u32) -> String {
    if pointer.is_null() || length == 0 || length == u32::MAX {
        return String::new();
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(pointer, length as usize) })
}
