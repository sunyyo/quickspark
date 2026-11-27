mod ai_credentials;
mod ai_window;
mod clipboard;
mod date;
mod http_url;
mod markdown;
mod shell;
mod theme;
mod window;

pub(crate) use ai_credentials::{
    ApiKeyPromptResult, ai_api_key_exists, load_ai_api_key, load_tavily_api_key, prompt_ai_api_key,
    prompt_tavily_api_key, tavily_api_key_exists,
};
pub(crate) use ai_window::{
    append_ai_answer, append_ai_reasoning, cancel_ai_answer, close_ai_answer, fail_ai_answer,
    finish_ai_answer, show_ai_answer, show_ai_stream, update_ai_status,
};
pub(crate) use clipboard::copy_text;
pub(crate) use date::local_date;
pub(crate) use http_url::ParsedUrl;
pub(crate) use shell::{known_folder_path, shell_execute, windows_program};
pub(crate) use theme::theme;
pub(crate) use window::activate_window;

pub(crate) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

pub(crate) fn copy_wide(value: &str, buffer: &mut [u16]) {
    if buffer.is_empty() {
        return;
    }
    let capacity = buffer.len() - 1;
    let mut written = 0;
    for (destination, source) in buffer.iter_mut().take(capacity).zip(value.encode_utf16()) {
        *destination = source;
        written += 1;
    }
    buffer[written] = 0;
}

pub(crate) fn scale_dpi(value: i32, dpi: u32) -> i32 {
    value * dpi as i32 / 96
}

pub(crate) fn create_ui_font(
    dpi: u32,
    pixels: i32,
    weight: u32,
) -> windows_sys::Win32::Graphics::Gdi::HFONT {
    use windows_sys::Win32::Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, FF_DONTCARE,
        OUT_DEFAULT_PRECIS,
    };
    unsafe {
        CreateFontW(
            scale_dpi(-pixels, dpi),
            0,
            0,
            0,
            weight as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            FF_DONTCARE as u32,
            wide("Microsoft YaHei UI").as_ptr(),
        )
    }
}

#[cfg(test)]
pub(crate) fn run_on_ui_thread<R: Send + 'static>(task: impl FnOnce() -> R + Send + 'static) -> R {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
        sync::{OnceLock, mpsc},
        thread,
    };

    type UiJob = Box<dyn FnOnce() + Send + 'static>;
    static UI_THREAD: OnceLock<mpsc::Sender<UiJob>> = OnceLock::new();

    let sender = UI_THREAD.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<UiJob>();
        thread::Builder::new()
            .name("quickspark-test-ui".to_owned())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    job();
                }
            })
            .expect("spawn persistent test UI thread");
        tx
    });
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    sender
        .send(Box::new(move || {
            let outcome = catch_unwind(AssertUnwindSafe(task));
            let _ = done_tx.send(outcome);
        }))
        .expect("send job to persistent test UI thread");
    match done_rx
        .recv()
        .expect("receive result from persistent test UI thread")
    {
        Ok(value) => value,
        Err(payload) => resume_unwind(payload),
    }
}
