use std::{
    ffi::c_void,
    net::IpAddr,
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use serde_json::{Value, json};
use windows_sys::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_REDIRECTS,
    WINHTTP_FLAG_SECURE, WINHTTP_INTERNET_SCHEME_HTTP, WINHTTP_INTERNET_SCHEME_HTTPS,
    WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
};

use crate::{
    model::{AiRequest, AiTask},
    platform::windows::wide,
    storage::{AiConfig, ThinkingMode},
};

use super::{AiResponse, StreamDelta, preview};

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_READ_CHUNK_BYTES: u32 = 64 * 1024;

pub(super) fn request_body(config: &AiConfig, request: &AiRequest) -> Vec<u8> {
    let mut messages = Vec::with_capacity(2);
    let system_prompt = match request.task {
        AiTask::Question => config.system_prompt.trim(),
        AiTask::Translation => "你是一名专业翻译，只执行翻译任务，并严格遵守用户给出的翻译规则。",
    };
    if !system_prompt.is_empty() {
        messages.push(json!({
            "role": "system",
            "content": system_prompt,
        }));
    }
    messages.push(json!({ "role": "user", "content": request.request_prompt }));
    let mut body = json!({
        "model": config.model.trim(),
        "messages": messages,
        "stream": true,
        "max_tokens": config.max_tokens,
    });
    if let Some(body) = body.as_object_mut() {
        match config.thinking_mode {
            ThinkingMode::Auto => {}
            ThinkingMode::Enabled => {
                body.insert("thinking".to_owned(), json!({ "type": "enabled" }));
                body.insert(
                    "reasoning_effort".to_owned(),
                    json!(config.reasoning_effort.key()),
                );
            }
            ThinkingMode::Disabled => {
                body.insert("thinking".to_owned(), json!({ "type": "disabled" }));
            }
        }
    }
    serde_json::to_vec(&body).unwrap_or_default()
}

pub(super) fn agent_request_body(config: &AiConfig, system: &str, prompt: &str) -> Vec<u8> {
    let messages = vec![
        json!({ "role": "system", "content": system }),
        json!({ "role": "user", "content": prompt }),
    ];
    let mut body = json!({
        "model": config.model.trim(),
        "messages": messages,
        "stream": false,
        "max_tokens": config.max_tokens.clamp(512, 4096),
    });
    if let Some(body) = body.as_object_mut() {
        match config.thinking_mode {
            ThinkingMode::Auto => {}
            ThinkingMode::Enabled => {
                body.insert("thinking".to_owned(), json!({ "type": "enabled" }));
                body.insert(
                    "reasoning_effort".to_owned(),
                    json!(config.reasoning_effort.key()),
                );
            }
            ThinkingMode::Disabled => {
                body.insert("thinking".to_owned(), json!({ "type": "disabled" }));
            }
        }
    }
    serde_json::to_vec(&body).unwrap_or_default()
}

pub(super) fn extract_response(bytes: &[u8]) -> Result<AiResponse, String> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("接口返回的不是有效 JSON：{error}"))?;
    if let Some(message) = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
    {
        return Err(message.to_owned());
    }
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .ok_or_else(|| "接口响应中缺少 choices[0]".to_owned())?;
    let message = choice.get("message");
    let content = message
        .and_then(|message| message.get("content"))
        .or_else(|| choice.get("text"))
        .ok_or_else(|| "接口响应中缺少回答内容".to_owned())?;
    let answer = content_text(content);
    let answer = answer.trim();
    if answer.is_empty() {
        Err("接口返回了空回答".to_owned())
    } else {
        let reasoning = message
            .and_then(|message| message.get("reasoning_content"))
            .map(content_text)
            .unwrap_or_default();
        Ok(AiResponse {
            answer: answer.to_owned(),
            reasoning,
            finish_reason: choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

pub(super) fn winhttp_stream_json<F>(
    config: &AiConfig,
    api_key: &str,
    body: &[u8],
    context: RequestContext,
    mut on_delta: F,
) -> Result<AiResponse, String>
where
    F: FnMut(StreamDelta),
{
    let request = post_json(
        &config.api_url,
        api_key,
        body,
        config.timeout_seconds,
        &context,
        "AI",
        "text/event-stream, application/json",
    )?;
    let status = response_status(request.handle.0)?;
    if !(200..300).contains(&status) {
        let response = read_response(request.handle.0)?;
        let details = api_error_message(&response);
        return Err(if details.is_empty() {
            format!("AI 接口返回 HTTP {status}")
        } else {
            format!("AI 接口返回 HTTP {status}：{details}")
        });
    }

    let mut raw_response = Vec::new();
    let mut decoder = SseDecoder::default();
    let mut answer = String::new();
    let mut reasoning = String::new();
    let mut finish_reason = None;
    let mut saw_sse = false;
    let mut done = false;
    read_response_chunks(request.handle.0, |chunk| {
        context.ensure_current()?;
        let new_len = raw_response
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "AI 响应过大".to_owned())?;
        if new_len > MAX_RESPONSE_BYTES {
            return Err("AI 响应超过 4 MB 安全上限".to_owned());
        }
        raw_response.extend_from_slice(chunk);
        for frame in decoder.push(chunk)? {
            saw_sse = true;
            match frame {
                StreamFrame::Reasoning(text) => {
                    reasoning.push_str(&text);
                    on_delta(StreamDelta::Reasoning(text));
                }
                StreamFrame::Answer(text) => {
                    answer.push_str(&text);
                    on_delta(StreamDelta::Answer(text));
                }
                StreamFrame::FinishReason(reason) => finish_reason = Some(reason),
                StreamFrame::Done => done = true,
            }
        }
        Ok(())
    })?;
    if !done {
        for frame in decoder.finish()? {
            saw_sse = true;
            match frame {
                StreamFrame::Reasoning(text) => {
                    reasoning.push_str(&text);
                    on_delta(StreamDelta::Reasoning(text));
                }
                StreamFrame::Answer(text) => {
                    answer.push_str(&text);
                    on_delta(StreamDelta::Answer(text));
                }
                StreamFrame::FinishReason(reason) => finish_reason = Some(reason),
                StreamFrame::Done => {}
            }
        }
    }
    if saw_sse {
        let answer = answer.trim();
        if answer.is_empty() {
            Err("接口结束了流式响应，但没有返回回答内容".to_owned())
        } else {
            Ok(AiResponse {
                answer: answer.to_owned(),
                reasoning,
                finish_reason,
            })
        }
    } else {
        let response = extract_response(&raw_response)?;
        if !response.reasoning.is_empty() {
            on_delta(StreamDelta::Reasoning(response.reasoning.clone()));
        }
        on_delta(StreamDelta::Answer(response.answer.clone()));
        Ok(response)
    }
}

pub(super) fn winhttp_json(
    url: &str,
    api_key: &str,
    body: &[u8],
    timeout_seconds: u32,
    context: RequestContext,
    service_name: &str,
) -> Result<Vec<u8>, String> {
    let request = post_json(
        url,
        api_key,
        body,
        timeout_seconds,
        &context,
        service_name,
        "application/json",
    )?;
    let status = response_status(request.handle.0)?;
    let mut response = Vec::new();
    read_response_chunks(request.handle.0, |chunk| {
        context.ensure_current()?;
        let new_len = response
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| format!("{service_name} 响应过大"))?;
        if new_len > MAX_RESPONSE_BYTES {
            return Err(format!("{service_name} 响应超过 4 MB 安全上限"));
        }
        response.extend_from_slice(chunk);
        Ok(())
    })?;
    if !(200..300).contains(&status) {
        let details = api_error_message(&response);
        return Err(if details.is_empty() {
            format!("{service_name} 接口返回 HTTP {status}")
        } else {
            format!("{service_name} 接口返回 HTTP {status}：{details}")
        });
    }
    Ok(response)
}

// Drop the request before its parent connection and session.
struct HttpRequest {
    handle: InternetHandle,
    _connection: InternetHandle,
    _session: InternetHandle,
}

fn post_json(
    url: &str,
    api_key: &str,
    body: &[u8],
    timeout_seconds: u32,
    context: &RequestContext,
    service_name: &str,
    accept: &str,
) -> Result<HttpRequest, String> {
    let parsed = ParsedUrl::parse(url)?;
    context.ensure_current()?;
    let user_agent = wide(concat!("QuickSpark/", env!("CARGO_PKG_VERSION")));
    let session = InternetHandle::new(unsafe {
        WinHttpOpen(
            user_agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        )
    })
    .ok_or_else(|| last_error("无法初始化 WinHTTP"))?;
    let timeout = i32::try_from(timeout_seconds.saturating_mul(1_000)).unwrap_or(i32::MAX);
    if unsafe { WinHttpSetTimeouts(session.0, timeout, timeout, timeout, timeout) } == 0 {
        return Err(last_error(&format!("无法设置 {service_name} 请求超时")));
    }
    let host = wide(&parsed.host);
    let connection =
        InternetHandle::new(unsafe { WinHttpConnect(session.0, host.as_ptr(), parsed.port, 0) })
            .ok_or_else(|| last_error(&format!("无法连接 {service_name} 接口主机")))?;
    let verb = wide("POST");
    let object = wide(&parsed.object);
    let flags = if parsed.secure {
        WINHTTP_FLAG_SECURE
    } else {
        0
    };
    let request = unsafe {
        WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            object.as_ptr(),
            null(),
            null(),
            null(),
            flags,
        )
    };
    let request = InternetHandle::new(request)
        .ok_or_else(|| last_error(&format!("无法创建 {service_name} HTTP 请求")))?;
    context.ensure_current()?;
    let disabled_features = WINHTTP_DISABLE_REDIRECTS;
    if unsafe {
        WinHttpSetOption(
            request.0,
            WINHTTP_OPTION_DISABLE_FEATURE,
            (&disabled_features as *const u32).cast(),
            std::mem::size_of_val(&disabled_features) as u32,
        )
    } == 0
    {
        return Err(last_error(&format!(
            "无法禁用 {service_name} 请求自动重定向"
        )));
    }
    let mut headers = format!("Content-Type: application/json\r\nAccept: {accept}\r\n");
    if !api_key.trim().is_empty() {
        headers.push_str("Authorization: Bearer ");
        headers.push_str(api_key.trim());
        headers.push_str("\r\n");
    }
    let headers = wide(&headers);
    if unsafe {
        WinHttpSendRequest(
            request.0,
            headers.as_ptr(),
            u32::MAX,
            body.as_ptr() as _,
            body.len() as u32,
            body.len() as u32,
            0,
        )
    } == 0
    {
        return Err(last_error(&format!("发送 {service_name} 请求失败")));
    }
    context.ensure_current()?;
    if unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0 {
        return Err(last_error(&format!("接收 {service_name} 响应失败")));
    }
    context.ensure_current()?;
    Ok(HttpRequest {
        handle: request,
        _connection: connection,
        _session: session,
    })
}

fn response_status(request: *mut c_void) -> Result<u32, String> {
    let mut status = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    if unsafe {
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut u32 as _,
            &mut size,
            null_mut(),
        )
    } == 0
    {
        return Err(last_error("无法读取 AI 接口状态码"));
    }
    Ok(status)
}

fn read_response(request: *mut c_void) -> Result<Vec<u8>, String> {
    let mut response = Vec::new();
    read_response_chunks(request, |chunk| {
        let new_len = response
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "AI 响应过大".to_owned())?;
        if new_len > MAX_RESPONSE_BYTES {
            return Err("AI 响应超过 4 MB 安全上限".to_owned());
        }
        response.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(response)
}

fn read_response_chunks<F>(request: *mut c_void, mut on_chunk: F) -> Result<(), String>
where
    F: FnMut(&[u8]) -> Result<(), String>,
{
    loop {
        let mut available = 0u32;
        if unsafe { WinHttpQueryDataAvailable(request, &mut available) } == 0 {
            return Err(last_error("读取 AI 响应长度失败"));
        }
        if available == 0 {
            break;
        }
        // Some servers can report a very large amount of buffered data. Read it in
        // bounded pieces so a single untrusted response cannot force one huge allocation.
        let requested = available.min(MAX_READ_CHUNK_BYTES);
        let mut chunk = vec![0u8; requested as usize];
        let mut read = 0u32;
        if unsafe { WinHttpReadData(request, chunk.as_mut_ptr() as _, requested, &mut read) } == 0 {
            return Err(last_error("读取 AI 响应内容失败"));
        }
        if read == 0 {
            break;
        }
        on_chunk(&chunk[..read as usize])?;
    }
    Ok(())
}

fn api_error_message(bytes: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes)
        && let Some(message) = value
            .get("error")
            .and_then(|error| error.get("message").or(Some(error)))
            .and_then(Value::as_str)
    {
        return preview(message, 240);
    }
    std::str::from_utf8(bytes)
        .ok()
        .map(|text| preview(text, 240))
        .unwrap_or_default()
}

#[derive(Clone)]
pub(super) struct RequestContext {
    pub(super) current_generation: Arc<AtomicU64>,
    pub(super) generation: u64,
}

impl RequestContext {
    #[cfg(test)]
    pub(super) fn standalone() -> Self {
        Self {
            current_generation: Arc::new(AtomicU64::new(1)),
            generation: 1,
        }
    }

    pub(super) fn ensure_current(&self) -> Result<(), String> {
        (self.current_generation.load(Ordering::Acquire) == self.generation)
            .then_some(())
            .ok_or_else(|| "AI 请求已取消".to_owned())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum StreamFrame {
    Reasoning(String),
    Answer(String),
    FinishReason(String),
    Done,
}

#[derive(Default)]
pub(super) struct SseDecoder {
    pending: Vec<u8>,
}

impl SseDecoder {
    pub(super) fn push(&mut self, chunk: &[u8]) -> Result<Vec<StreamFrame>, String> {
        self.pending.extend_from_slice(chunk);
        let mut frames = Vec::new();
        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line = self.pending.drain(..=newline).collect::<Vec<_>>();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            frames.extend(parse_sse_line(&line)?);
        }
        Ok(frames)
    }

    pub(super) fn finish(&mut self) -> Result<Vec<StreamFrame>, String> {
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        let line = std::mem::take(&mut self.pending);
        parse_sse_line(&line)
    }
}

fn parse_sse_line(line: &[u8]) -> Result<Vec<StreamFrame>, String> {
    let Some(data) = line.strip_prefix(b"data:") else {
        return Ok(Vec::new());
    };
    let data = trim_ascii(data);
    if data == b"[DONE]" {
        return Ok(vec![StreamFrame::Done]);
    }
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let value: Value =
        serde_json::from_slice(data).map_err(|error| format!("无法解析 AI 流式数据：{error}"))?;
    if let Some(message) = value
        .get("error")
        .and_then(|error| error.get("message").or(Some(error)))
        .and_then(Value::as_str)
    {
        return Err(message.to_owned());
    }
    let Some(choice) = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
    else {
        return Ok(Vec::new());
    };
    let delta = choice.get("delta").or_else(|| choice.get("message"));
    let mut frames = Vec::with_capacity(3);
    if let Some(reasoning) = delta
        .and_then(|delta| delta.get("reasoning_content"))
        .map(content_text)
        .filter(|text| !text.is_empty())
    {
        frames.push(StreamFrame::Reasoning(reasoning));
    }
    if let Some(content) = delta
        .and_then(|delta| delta.get("content"))
        .or_else(|| choice.get("text"))
        .map(content_text)
        .filter(|text| !text.is_empty())
    {
        frames.push(StreamFrame::Answer(content));
    }
    if let Some(reason) = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.is_empty())
    {
        frames.push(StreamFrame::FinishReason(reason.to_owned()));
    }
    Ok(frames)
}

fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.as_str()
                    .or_else(|| part.get("text").and_then(Value::as_str))
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn trim_ascii(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

pub(super) struct ParsedUrl {
    pub(super) host: String,
    pub(super) object: String,
    pub(super) port: u16,
    secure: bool,
}

impl ParsedUrl {
    pub(super) fn parse(url: &str) -> Result<Self, String> {
        let wide_url = wide(url.trim());
        let mut components = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
            dwSchemeLength: u32::MAX,
            dwHostNameLength: u32::MAX,
            dwUrlPathLength: u32::MAX,
            dwExtraInfoLength: u32::MAX,
            ..Default::default()
        };
        if unsafe { WinHttpCrackUrl(wide_url.as_ptr(), 0, 0, &mut components) } == 0 {
            return Err(last_error("AI API 地址无效"));
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

struct InternetHandle(*mut c_void);

impl InternetHandle {
    fn new(handle: *mut c_void) -> Option<Self> {
        (!handle.is_null()).then_some(Self(handle))
    }
}

impl Drop for InternetHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn last_error(context: &str) -> String {
    format!("{context}：{}", std::io::Error::last_os_error())
}
