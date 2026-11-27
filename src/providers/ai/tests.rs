use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Mutex, atomic::AtomicU64},
    thread,
    time::Duration,
};

use super::{
    AiResponse, AiState, StreamDelta, ai_query, finish_event_if_current, lock, preview,
    question_request, request_cache_key, translation_query, translation_request,
    transport::{
        RequestContext, SseDecoder, StreamFrame, agent_request_body, extract_response,
        request_body, winhttp_json, winhttp_stream_json,
    },
};
use crate::model::{AiCompletion, WebSearchMode};
use crate::platform::windows::ParsedUrl;
use crate::storage::{AiConfig, ReasoningEffort, ThinkingMode};

fn read_http_request(stream: &mut impl Read) -> Vec<u8> {
    let mut request = Vec::new();
    let header_end = loop {
        let mut chunk = [0u8; 4 * 1024];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0);
        request.extend_from_slice(&chunk[..read]);
        if let Some(position) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let header_text = String::from_utf8_lossy(&request[..header_end]);
    let content_length = header_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap();
    while request.len() < header_end + content_length {
        let mut chunk = [0u8; 4 * 1024];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0);
        request.extend_from_slice(&chunk[..read]);
    }
    request
}

#[test]
fn recognizes_only_explicit_ai_prefix() {
    assert_eq!(ai_query("ai"), Some(""));
    assert_eq!(ai_query("AI 什么是 Rust"), Some("什么是 Rust"));
    assert_eq!(ai_query("aid"), None);
    assert_eq!(ai_query("air quality"), None);
}

#[test]
fn parses_question_web_search_overrides() {
    let automatic = question_request("最新 Rust 版本");
    assert_eq!(automatic.web_search, WebSearchMode::Auto);
    assert_eq!(automatic.request_prompt, "最新 Rust 版本");

    let required = question_request("--web 最新 Rust 版本");
    assert_eq!(required.web_search, WebSearchMode::Required);
    assert_eq!(required.request_prompt, "最新 Rust 版本");

    let disabled = question_request("--no-web Rust 所有权");
    assert_eq!(disabled.web_search, WebSearchMode::Disabled);
    assert_eq!(disabled.request_prompt, "Rust 所有权");

    assert_eq!(question_request("--web").request_prompt, "--web");
}

#[test]
fn recognizes_translation_prefixes_without_stealing_regular_searches() {
    assert_eq!(translation_query("tr"), Some(""));
    assert_eq!(translation_query("TR hello"), Some("hello"));
    assert_eq!(translation_query("translate hello"), Some("hello"));
    assert_eq!(translation_query("fy 你好"), Some("你好"));
    assert_eq!(translation_query("翻译 hello"), Some("hello"));
    assert_eq!(translation_query("tree"), None);
    assert_eq!(translation_query("translateable"), None);
}

#[test]
fn builds_automatic_and_explicit_translation_requests() {
    let automatic = translation_request("你好，世界").unwrap();
    assert_eq!(automatic.display_prompt, "自动识别语言并翻译：你好，世界");
    assert!(automatic.request_prompt.contains("主要语言"));
    assert!(automatic.request_prompt.contains("不是中文"));
    assert!(automatic.request_prompt.contains("简体中文"));
    assert!(automatic.request_prompt.contains("只输出译文"));
    assert!(automatic.request_prompt.contains("母语者习惯"));
    assert!(automatic.request_prompt.contains("避免逐字直译"));
    assert!(automatic.request_prompt.contains("保持原文的语气"));
    assert_eq!(automatic.task.label(), "AI 翻译");

    let japanese = translation_request("日本語の文章です").unwrap();
    assert_eq!(
        japanese.display_prompt,
        "自动识别语言并翻译：日本語の文章です"
    );
    assert_eq!(
        japanese.request_prompt,
        automatic
            .request_prompt
            .replace("你好，世界", "日本語の文章です")
    );

    let explicit = translation_request("--to ja Good morning").unwrap();
    assert_eq!(explicit.display_prompt, "翻译为日语：Good morning");
    assert!(explicit.request_prompt.contains("翻译成日语"));
    assert_eq!(
        translation_request("en Good morning")
            .unwrap()
            .display_prompt,
        "自动识别语言并翻译：en Good morning"
    );
    assert!(translation_request("--to en").is_none());
}

#[test]
fn builds_openai_compatible_request_without_leaking_configuration() {
    let config = AiConfig {
        api_url: "https://example.com/v1/chat/completions".to_owned(),
        model: "demo-model".to_owned(),
        system_prompt: "简洁回答".to_owned(),
        max_tokens: 512,
        timeout_seconds: 30,
        ..AiConfig::default()
    };
    let question = question_request("你好");
    let body = request_body(&config, &question);
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("demo-model"));
    assert!(text.contains("你好"));
    assert!(text.contains("\"stream\":true"));
    assert!(!text.contains("\"thinking\""));
    assert!(!text.contains("example.com"));
    let planner = agent_request_body(&config, "controller", "decide");
    let planner: serde_json::Value = serde_json::from_slice(&planner).unwrap();
    assert_eq!(planner["stream"], false);
    assert_eq!(planner["messages"][0]["content"], "controller");
    assert_eq!(planner["messages"][1]["content"], "decide");

    let mut thinking = config.clone();
    thinking.thinking_mode = ThinkingMode::Enabled;
    thinking.reasoning_effort = ReasoningEffort::Max;
    let text = String::from_utf8(request_body(&thinking, &question)).unwrap();
    assert!(text.contains("\"thinking\":{\"type\":\"enabled\"}"));
    assert!(text.contains("\"reasoning_effort\":\"max\""));
    let planner = String::from_utf8(agent_request_body(&thinking, "controller", "decide")).unwrap();
    assert!(planner.contains("\"thinking\":{\"type\":\"disabled\"}"));
    assert!(!planner.contains("reasoning_effort"));

    thinking.thinking_mode = ThinkingMode::Disabled;
    let text = String::from_utf8(request_body(&thinking, &question)).unwrap();
    assert!(text.contains("\"thinking\":{\"type\":\"disabled\"}"));
    assert!(!text.contains("reasoning_effort"));

    let translation = translation_request("你好").unwrap();
    let text = String::from_utf8(request_body(&config, &translation)).unwrap();
    assert!(text.contains("专业翻译"));
    assert!(!text.contains("简洁回答"));
}

#[test]
fn cache_key_changes_with_behavior_affecting_configuration() {
    let request = question_request("相同问题");
    let config = AiConfig {
        api_url: "https://example.com/v1/chat/completions".to_owned(),
        model: "model-a".to_owned(),
        ..AiConfig::default()
    };
    let first = request_cache_key(&request, &config);
    let mut changed = config.clone();
    changed.model = "model-b".to_owned();
    assert_ne!(first, request_cache_key(&request, &changed));
    changed = config.clone();
    changed.max_tokens += 1;
    assert_ne!(first, request_cache_key(&request, &changed));
}

#[test]
fn extracts_string_and_structured_answers_and_api_errors() {
    assert_eq!(
        extract_response(
            r#"{"choices":[{"finish_reason":"length","message":{"reasoning_content":"思考","content":"回答"}}]}"#
                .as_bytes()
        )
        .unwrap(),
        super::AiResponse {
            answer: "回答".to_owned(),
            reasoning: "思考".to_owned(),
            completion: AiCompletion::LengthLimit,
        }
    );
    assert_eq!(
        extract_response(br#"{"choices":[{"message":{"content":[{"text":"A"},{"text":"B"}]}}]}"#)
            .unwrap(),
        super::AiResponse {
            answer: "AB".to_owned(),
            reasoning: String::new(),
            completion: AiCompletion::Complete,
        }
    );
    assert_eq!(
        extract_response(br#"{"error":{"message":"bad key"}}"#).unwrap_err(),
        "bad key"
    );
    assert_eq!(
        extract_response(br#"{"error":"flat error"}"#).unwrap_err(),
        "flat error"
    );
}

#[test]
fn parses_http_urls_and_truncates_unicode_preview() {
    let url = ParsedUrl::parse("https://localhost:11434/v1/chat/completions?x=1").unwrap();
    assert_eq!(url.host, "localhost");
    assert_eq!(url.port, 11434);
    assert_eq!(url.object, "/v1/chat/completions?x=1");
    assert!(ParsedUrl::parse("http://127.0.0.1:11434/v1/chat/completions").is_ok());
    assert!(ParsedUrl::parse("http://[::1]:11434/v1/chat/completions").is_ok());
    assert!(ParsedUrl::parse("http://example.com/v1/chat/completions").is_err());
    assert_eq!(preview("一二三四五", 3), "一二三…");
}

#[test]
fn cancelled_request_context_stops_before_opening_a_connection() {
    let config = AiConfig {
        api_url: "http://127.0.0.1:1/v1/chat/completions".to_owned(),
        model: "local-test".to_owned(),
        system_prompt: String::new(),
        max_tokens: 32,
        timeout_seconds: 5,
        ..AiConfig::default()
    };
    let request = request_body(&config, &question_request("测试取消"));
    let context = RequestContext::standalone();
    context
        .current_generation
        .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    let error = winhttp_stream_json(&config, "", &request, &context, |_| {}).unwrap_err();
    assert_eq!(error, "AI 请求已取消");
}

#[test]
fn stale_completion_cannot_overwrite_a_newer_loading_request() {
    let old_request = question_request("旧问题");
    let state = Mutex::new(AiState::Loading {
        generation: 2,
        cache_key: "new-key".to_owned(),
    });
    let generation = AtomicU64::new(2);

    let event = finish_event_if_current(
        &state,
        &generation,
        1,
        "old-key".to_owned(),
        old_request,
        Ok(AiResponse {
            answer: "迟到的回答".to_owned(),
            reasoning: String::new(),
            completion: AiCompletion::Complete,
        }),
    );

    assert!(event.is_none());
    let state = lock(&state);
    assert!(matches!(
        &*state,
        AiState::Loading {
            generation: 2,
            cache_key,
        } if cache_key == "new-key"
    ));
}

#[test]
fn terminal_events_keep_full_results_in_the_service_cache() {
    let request = question_request("保留原问题");
    let generation = AtomicU64::new(7);
    let response = AiResponse {
        answer: "完整回答".repeat(10_000),
        reasoning: "完整思考".repeat(10_000),
        completion: AiCompletion::LengthLimit,
    };
    for result in [Ok(response.clone()), Err("连接中断".to_owned())] {
        let state = Mutex::new(AiState::Loading {
            generation: 7,
            cache_key: "key-7".to_owned(),
        });
        let event = finish_event_if_current(
            &state,
            &generation,
            7,
            "key-7".to_owned(),
            request.clone(),
            result.clone(),
        )
        .unwrap();
        match (&event, &result) {
            (
                super::AiEvent::Finished {
                    generation: 7,
                    completion,
                },
                Ok(response),
            ) => assert_eq!(completion, &response.completion),
            (
                super::AiEvent::Failed {
                    generation: 7,
                    error,
                },
                Err(expected),
            ) => assert_eq!(error, expected),
            _ => panic!("unexpected terminal event: {event:?}"),
        }
        assert!(
            matches!(&*lock(&state), AiState::Finished { cache_key, request: saved, result: cached } if cache_key == "key-7" && saved == &request && cached == &result)
        );
        assert!(
            finish_event_if_current(
                &state,
                &generation,
                7,
                "key-7".to_owned(),
                request.clone(),
                result
            )
            .is_none()
        );
    }
}

#[test]
fn answer_actions_preserve_borrowed_full_text_and_regeneration_request() {
    let request = translation_request("--to en 你好").unwrap();
    let response = AiResponse {
        answer: "Hello. ".repeat(10_000),
        reasoning: "翻译依据".repeat(10_000),
        completion: AiCompletion::Complete,
    };
    let result = super::answer_result(&request, &response);
    assert!(
        matches!(&result.actions[0], crate::model::Action::ShowAiAnswer { prompt, answer, reasoning, completion }
        if *completion == response.completion && prompt == &request.display_prompt && answer == &response.answer && reasoning == &response.reasoning)
    );
    assert!(
        matches!(&result.actions[1], crate::model::Action::CopyText { text } if text == &response.answer)
    );
    assert!(
        matches!(&result.actions[2], crate::model::Action::RegenerateAi { request: saved } if saved == &request)
    );
}

#[test]
fn decodes_sse_across_arbitrary_network_chunks() {
    let mut decoder = SseDecoder::default();
    assert!(
        decoder
            .push(b"data: {\"choices\":[{\"delta\":{\"cont")
            .unwrap()
            .is_empty()
    );
    let frames = decoder
        .push(
            "ent\":\"你好\",\"reasoning_content\":\"先想\"},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\n"
                .as_bytes(),
        )
        .unwrap();
    assert_eq!(
        frames,
        vec![
            StreamFrame::Reasoning("先想".to_owned()),
            StreamFrame::Answer("你好".to_owned()),
            StreamFrame::FinishReason("stop".to_owned()),
            StreamFrame::Done,
        ]
    );
}

struct TestHttpServer {
    address: std::net::SocketAddr,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestHttpServer {
    fn spawn(handler: impl FnOnce(std::net::TcpStream) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled_flag = std::sync::Arc::clone(&cancelled);
        let handle = thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            if cancelled_flag.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
            handler(stream);
        });
        Self {
            address,
            cancelled,
            handle: Some(handle),
        }
    }

    fn address(&self) -> std::net::SocketAddr {
        self.address
    }

    fn join(mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

impl Drop for TestHttpServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            self.cancelled
                .store(true, std::sync::atomic::Ordering::Release);
            let _ = std::net::TcpStream::connect_timeout(&self.address, Duration::from_millis(200));
            let _ = handle.join();
        }
    }
}

#[test]
fn posts_to_a_local_openai_compatible_endpoint() {
    for (ending, expected) in [
        ("data: [DONE]\n\n", AiCompletion::Complete),
        ("data: [DONE]", AiCompletion::Complete),
        (
            "data: {\"choices\":[{\"finish_reason\":\"length\"}]}\n\n",
            AiCompletion::LengthLimit,
        ),
        ("", AiCompletion::Interrupted),
        (
            "data: {\"choices\":[{\"delta\":{\"content\":\"残缺",
            AiCompletion::Interrupted,
        ),
    ] {
        let server = TestHttpServer::spawn(move |mut stream| {
            let request = read_http_request(&mut stream);
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
            assert!(request.contains(concat!(
                "User-Agent: QuickSpark/",
                env!("CARGO_PKG_VERSION")
            )));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-key")
            );
            let deltas = concat!(
                "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"分析\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"本地\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"测试成功\"}}]}\n\n",
            );
            let body = format!("{deltas}{ending}");
            let size = body.len();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n{body}"
            )
            .unwrap();
            stream.flush().unwrap();
        });
        let config = AiConfig {
            api_url: format!("http://{}/v1/chat/completions", server.address()),
            model: "local-test".to_owned(),
            system_prompt: String::new(),
            max_tokens: 32,
            timeout_seconds: 5,
            ..AiConfig::default()
        };
        let request = request_body(&config, &question_request("测试"));
        let mut deltas = Vec::new();
        let answer = winhttp_stream_json(
            &config,
            "test-key",
            &request,
            &RequestContext::standalone(),
            |delta| deltas.push(delta),
        )
        .unwrap();
        assert_eq!(answer.answer, "本地测试成功");
        assert_eq!(answer.reasoning, "分析");
        assert_eq!(answer.completion, expected);
        let cached = super::answer_result(&question_request("测试"), &answer);
        assert!(
            matches!(&cached.actions[0], crate::model::Action::ShowAiAnswer { answer, completion, .. }
        if answer == "本地测试成功" && *completion == expected)
        );
        assert!(
            matches!(&cached.actions[1], crate::model::Action::CopyText { text } if text == "本地测试成功")
        );
        assert_eq!(
            deltas,
            [
                StreamDelta::Reasoning("分析".to_owned()),
                StreamDelta::Answer("本地".to_owned()),
                StreamDelta::Answer("测试成功".to_owned()),
            ]
        );
        server.join();
    }
}

#[test]
fn posts_bearer_authenticated_json_to_a_local_search_endpoint() {
    let server = TestHttpServer::spawn(|mut stream| {
        let request = read_http_request(&mut stream);
        let request = String::from_utf8_lossy(&request);
        assert!(request.starts_with("POST /search HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer tavily-test-key")
        );
        assert!(request.contains(r#"{"query":"rust"}"#));
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 14\r\nConnection: close\r\n\r\n{\"results\":[]}",
            )
            .unwrap();
    });

    let response = winhttp_json(
        &format!("http://{}/search", server.address()),
        "tavily-test-key",
        br#"{"query":"rust"}"#,
        5,
        &RequestContext::standalone(),
        "Tavily",
    )
    .unwrap();
    assert_eq!(response, br#"{"results":[]}"#);
    server.join();
}

#[test]
fn refuses_redirects_instead_of_forwarding_bearer_credentials() {
    for streaming in [true, false] {
        let redirected = TcpListener::bind("127.0.0.1:0").unwrap();
        redirected.set_nonblocking(true).unwrap();
        let redirected_address = redirected.local_addr().unwrap();
        let server = TestHttpServer::spawn(move |mut stream| {
            let request = read_http_request(&mut stream);
            assert!(
                String::from_utf8_lossy(&request)
                    .to_ascii_lowercase()
                    .contains("authorization: bearer redirect-secret")
            );
            write!(
                stream,
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{redirected_address}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            stream.flush().unwrap();
        });

        let config = AiConfig {
            api_url: format!("http://{}/v1/chat/completions", server.address()),
            model: "local-test".to_owned(),
            system_prompt: String::new(),
            max_tokens: 32,
            timeout_seconds: 5,
            ..AiConfig::default()
        };
        let request = request_body(&config, &question_request("敏感问题"));
        let error = if streaming {
            winhttp_stream_json(
                &config,
                "redirect-secret",
                &request,
                &RequestContext::standalone(),
                |_| {},
            )
            .unwrap_err()
        } else {
            winhttp_json(
                &config.api_url,
                "redirect-secret",
                &request,
                5,
                &RequestContext::standalone(),
                "Tavily",
            )
            .unwrap_err()
        };
        assert!(error.contains("HTTP 307"), "unexpected error: {error}");
        server.join();
        assert!(matches!(
            redirected.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
    }
}

#[test]
fn streaming_sse_enforces_decoded_limit_instead_of_4mb_wire_envelope() {
    let frame = format!(
        "data: {{\"id\":\"chatcmpl-0123456789abcdef\",\"object\":\"chat.completion.chunk\",\"created\":1731900000,\"model\":\"deepseek-reasoner\",\"system_fingerprint\":\"{}\",\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"a\"}},\"logprobs\":null,\"finish_reason\":null}}]}}\n\n",
        "x".repeat(80)
    );
    let repeat = (4 * 1024 * 1024 / frame.len()) + 50;
    let server = TestHttpServer::spawn(move |mut stream| {
        let _ = read_http_request(&mut stream);
        // Each SSE frame has ~310 bytes of JSON envelope around a 1-byte token,
        // exceeding 4 MB raw wire traffic while decoded text stays tiny (~14 KB).
        let done =
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let total_size = frame.len() * repeat + done.len();
        assert!(total_size > 4 * 1024 * 1024);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {total_size}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let batch = frame.repeat(128);
        let mut remaining = repeat;
        while remaining >= 128 {
            stream.write_all(batch.as_bytes()).unwrap();
            remaining -= 128;
        }
        for _ in 0..remaining {
            stream.write_all(frame.as_bytes()).unwrap();
        }
        stream.write_all(done.as_bytes()).unwrap();
        stream.flush().unwrap();
    });

    let config = AiConfig {
        api_url: format!("http://{}/v1/chat/completions", server.address()),
        model: "local-test".to_owned(),
        system_prompt: String::new(),
        max_tokens: 32,
        timeout_seconds: 10,
        ..AiConfig::default()
    };
    let body = request_body(&config, &question_request("长流测试"));
    let response = winhttp_stream_json(
        &config,
        "test-key",
        &body,
        &RequestContext::standalone(),
        |_| {},
    )
    .unwrap();
    assert_eq!(response.completion, AiCompletion::Complete);
    assert_eq!(response.answer.len(), repeat);
    server.join();
}

#[test]
fn rejects_control_characters_in_bearer_api_key() {
    let config = AiConfig {
        api_url: "http://127.0.0.1:1/v1/chat/completions".to_owned(),
        model: "local-test".to_owned(),
        ..AiConfig::default()
    };
    let body = request_body(&config, &question_request("测试"));
    let error = winhttp_stream_json(
        &config,
        "bad-key\r\nX-Injected: 1",
        &body,
        &RequestContext::standalone(),
        |_| {},
    )
    .unwrap_err();
    assert!(error.contains("控制字符"), "unexpected error: {error}");
}
