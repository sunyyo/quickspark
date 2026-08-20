use std::{
    collections::HashSet,
    path::PathBuf,
    ptr::{null, null_mut},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::{DataExchange::COPYDATASTRUCT, LibraryLoader::GetModuleHandleW},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW,
        GWLP_USERDATA, GetWindowLongPtrW, HWND_MESSAGE, MSG, PM_REMOVE, PeekMessageW, PostMessageW,
        RegisterClassW, SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT, SendMessageTimeoutW, SetWindowLongPtrW,
        TranslateMessage, WM_COPYDATA, WNDCLASSW,
    },
};

use crate::{
    model::{Action, Query, ResultItem},
    platform::windows::wide,
};

use super::{Provider, ProviderContext, ProviderOutput};

const PROVIDER_ID: &str = "file-search";
const RESULT_LIMIT: usize = 20;
const DEBOUNCE: Duration = Duration::from_millis(150);
const EVERYTHING_REPLY_TIMEOUT: Duration = Duration::from_secs(2);
const EVERYTHING_WINDOW_CLASS: &str = "EVERYTHING_TASKBAR_NOTIFICATION";
const REPLY_WINDOW_CLASS: &str = "QuickSparkEverythingReplyWindow";
const EVERYTHING_COPYDATA_QUERY2W: usize = 18;
const EVERYTHING_REQUEST_FULL_PATH_AND_NAME: u32 = 0x0000_0004;
const EVERYTHING_SORT_NAME_ASCENDING: u32 = 1;
const REPLY_TOKEN_START: u32 = 0x5153_0001;
const QUERY2_HEADER_SIZE: usize = 7 * size_of::<u32>();
const LIST2_HEADER_SIZE: usize = 5 * size_of::<u32>();
const ITEM2_SIZE: usize = 2 * size_of::<u32>();

pub(crate) struct FileSearchProvider {
    requests: Sender<SearchRequest>,
    state: Arc<Mutex<ProviderState>>,
    latest_generation: Arc<AtomicU64>,
}

impl FileSearchProvider {
    pub(crate) fn new(window: usize, ready_message: u32) -> Self {
        let (requests, receiver) = mpsc::channel();
        let state = Arc::new(Mutex::new(ProviderState::default()));
        let latest_generation = Arc::new(AtomicU64::new(0));
        let worker_state = Arc::clone(&state);
        let worker_generation = Arc::clone(&latest_generation);
        thread::spawn(move || {
            search_worker(
                receiver,
                worker_state,
                worker_generation,
                window,
                ready_message,
            )
        });
        Self {
            requests,
            state,
            latest_generation,
        }
    }

    fn invalidate(&self) {
        let mut state = lock_state(&self.state);
        if state.requested.is_some() || state.completed.is_some() {
            state.requested = None;
            state.completed = None;
            self.latest_generation.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn request(&self, terms: &str) -> Option<SearchOutcome> {
        let mut state = lock_state(&self.state);
        if state.requested.as_deref() != Some(terms) {
            let generation = self.latest_generation.fetch_add(1, Ordering::AcqRel) + 1;
            state.requested = Some(terms.to_owned());
            state.completed = None;
            let _ = self.requests.send(SearchRequest {
                generation,
                terms: terms.to_owned(),
            });
        }
        state
            .completed
            .as_ref()
            .filter(|completed| completed.terms == terms)
            .map(|completed| completed.outcome.clone())
    }
}

impl Provider for FileSearchProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn query(&self, query: &Query, _context: &ProviderContext<'_>) -> ProviderOutput {
        let Some(terms) = file_query(query.raw()) else {
            self.invalidate();
            return ProviderOutput::empty();
        };

        let terms = terms.trim();
        if terms.is_empty() {
            self.invalidate();
            return exclusive_message("输入 f 关键词通过 Everything 搜索文件");
        }
        if terms.chars().count() < 2 {
            self.invalidate();
            return exclusive_message("请至少输入 2 个字符");
        }

        match self.request(terms) {
            None => exclusive_message("正在通过 Everything 搜索…"),
            Some(SearchOutcome::Unavailable) => {
                exclusive_message("Everything 不可用，请安装或启动 Everything")
            }
            Some(SearchOutcome::Entries(entries)) if entries.is_empty() => {
                exclusive_message(&format!("Everything 未找到“{terms}”"))
            }
            Some(SearchOutcome::Entries(entries)) => ProviderOutput {
                results: entries.into_iter().map(ResultItem::from).collect(),
                exclusive: true,
            },
        }
    }
}

fn exclusive_message(message: &str) -> ProviderOutput {
    ProviderOutput {
        results: vec![ResultItem {
            provider_id: PROVIDER_ID,
            title: message.to_owned(),
            score: 100_000,
            actions: Vec::new(),
        }],
        exclusive: true,
    }
}

fn file_query(input: &str) -> Option<&str> {
    let input = input.trim();
    if input.eq_ignore_ascii_case("f") || input.eq_ignore_ascii_case("file") {
        return Some("");
    }
    let split_at = input.find(char::is_whitespace)?;
    let (prefix, terms) = input.split_at(split_at);
    (prefix.eq_ignore_ascii_case("f") || prefix.eq_ignore_ascii_case("file"))
        .then_some(terms.trim())
}

#[derive(Default)]
struct ProviderState {
    requested: Option<String>,
    completed: Option<CompletedSearch>,
}

struct CompletedSearch {
    terms: String,
    outcome: SearchOutcome,
}

#[derive(Clone)]
enum SearchOutcome {
    Entries(Vec<FileEntry>),
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileEntry {
    name: String,
    path: PathBuf,
}

impl From<FileEntry> for ResultItem {
    fn from(entry: FileEntry) -> Self {
        let parent = entry
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.to_string_lossy().into_owned());
        let title = parent.map_or_else(
            || entry.name.clone(),
            |parent| format!("{}  —  {parent}", entry.name),
        );
        let path_text = entry.path.to_string_lossy().into_owned();
        ResultItem {
            provider_id: PROVIDER_ID,
            title,
            score: 100_000,
            actions: vec![
                Action::OpenPath {
                    path: entry.path.clone(),
                },
                Action::OpenContainingFolder {
                    path: entry.path.clone(),
                },
                Action::CopyText { text: path_text },
            ],
        }
    }
}

struct SearchRequest {
    generation: u64,
    terms: String,
}

fn search_worker(
    receiver: Receiver<SearchRequest>,
    state: Arc<Mutex<ProviderState>>,
    latest_generation: Arc<AtomicU64>,
    window: usize,
    ready_message: u32,
) {
    let mut client = EverythingClient::new();
    while let Ok(mut request) = receiver.recv() {
        loop {
            match receiver.recv_timeout(DEBOUNCE) {
                Ok(newer) => request = newer,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }

        if latest_generation.load(Ordering::Acquire) != request.generation {
            continue;
        }
        if client.is_none() {
            client = EverythingClient::new();
        }
        let outcome = match client
            .as_mut()
            .map(|everything| everything.query(&request.terms, RESULT_LIMIT))
        {
            Some(Ok(entries)) => SearchOutcome::Entries(entries),
            Some(Err(_)) | None => SearchOutcome::Unavailable,
        };

        if latest_generation.load(Ordering::Acquire) != request.generation {
            continue;
        }
        let mut provider_state = lock_state(&state);
        if provider_state.requested.as_deref() != Some(request.terms.as_str()) {
            continue;
        }
        provider_state.completed = Some(CompletedSearch {
            terms: request.terms,
            outcome,
        });
        drop(provider_state);
        unsafe {
            PostMessageW(window as _, ready_message, 0, 0);
        }
    }
}

fn lock_state(state: &Mutex<ProviderState>) -> std::sync::MutexGuard<'_, ProviderState> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EverythingError {
    Unavailable,
    QueryRejected,
    ReplyTimedOut,
    InvalidReply,
}

#[derive(Default)]
struct ReplyContext {
    expected_token: usize,
    outcome: Option<Result<Vec<FileEntry>, EverythingError>>,
}

struct EverythingClient {
    hwnd: HWND,
    reply: Box<ReplyContext>,
    next_reply_token: u32,
}

impl EverythingClient {
    fn new() -> Option<Self> {
        let mut reply = Box::<ReplyContext>::default();
        let class_name = wide(REPLY_WINDOW_CLASS);
        let instance = unsafe { GetModuleHandleW(null()) };
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(everything_reply_window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..unsafe { std::mem::zeroed() }
        };
        unsafe {
            RegisterClassW(&window_class);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                instance,
                null(),
            )
        };
        if hwnd.is_null() {
            return None;
        }
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                reply.as_mut() as *mut ReplyContext as isize,
            );
        }
        Some(Self {
            hwnd,
            reply,
            next_reply_token: REPLY_TOKEN_START,
        })
    }

    fn query(&mut self, terms: &str, limit: usize) -> Result<Vec<FileEntry>, EverythingError> {
        let everything_class = wide(EVERYTHING_WINDOW_CLASS);
        let everything_window = unsafe { FindWindowW(everything_class.as_ptr(), null()) };
        if everything_window.is_null() {
            return Err(EverythingError::Unavailable);
        }

        let reply_token = self.next_reply_token;
        self.next_reply_token = self.next_reply_token.wrapping_add(1).max(REPLY_TOKEN_START);
        self.reply.expected_token = reply_token as usize;
        self.reply.outcome = None;

        let mut query = build_everything_query(
            self.hwnd as usize as u32,
            reply_token,
            terms,
            limit.clamp(1, RESULT_LIMIT) as u32,
        );
        let mut copy_data = COPYDATASTRUCT {
            dwData: EVERYTHING_COPYDATA_QUERY2W,
            cbData: query.len() as u32,
            lpData: query.as_mut_ptr().cast(),
        };
        let mut message_result = 0usize;
        let sent = unsafe {
            SendMessageTimeoutW(
                everything_window,
                WM_COPYDATA,
                self.hwnd as WPARAM,
                (&mut copy_data as *mut COPYDATASTRUCT) as LPARAM,
                SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
                EVERYTHING_REPLY_TIMEOUT.as_millis() as u32,
                &mut message_result,
            )
        };
        if sent == 0 || message_result == 0 {
            self.reply.expected_token = 0;
            return Err(EverythingError::QueryRejected);
        }

        let deadline = Instant::now() + EVERYTHING_REPLY_TIMEOUT;
        while self.reply.outcome.is_none() && Instant::now() < deadline {
            self.pump_messages();
            if self.reply.outcome.is_none() {
                thread::sleep(Duration::from_millis(1));
            }
        }
        self.reply.expected_token = 0;
        self.reply
            .outcome
            .take()
            .unwrap_or(Err(EverythingError::ReplyTimedOut))
    }

    fn pump_messages(&self) {
        let mut message: MSG = unsafe { std::mem::zeroed() };
        unsafe {
            while PeekMessageW(&mut message, self.hwnd, 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

impl Drop for EverythingClient {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn everything_reply_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_COPYDATA && lparam != 0 {
        let context = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ReplyContext };
        let copy_data = unsafe { &*(lparam as *const COPYDATASTRUCT) };
        if !context.is_null()
            && unsafe { (*context).expected_token } != 0
            && copy_data.dwData == unsafe { (*context).expected_token }
        {
            let outcome = if copy_data.lpData.is_null() {
                Err(EverythingError::InvalidReply)
            } else {
                let bytes = unsafe {
                    std::slice::from_raw_parts(
                        copy_data.lpData.cast::<u8>(),
                        copy_data.cbData as usize,
                    )
                };
                parse_everything_reply(bytes)
            };
            unsafe {
                (*context).outcome = Some(outcome);
            }
            return 1;
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn build_everything_query(reply_hwnd: u32, reply_token: u32, terms: &str, limit: u32) -> Vec<u8> {
    let search: Vec<u16> = terms.encode_utf16().chain(Some(0)).collect();
    let mut query = Vec::with_capacity(QUERY2_HEADER_SIZE + search.len() * size_of::<u16>());
    for value in [
        reply_hwnd,
        reply_token,
        0,
        0,
        limit,
        EVERYTHING_REQUEST_FULL_PATH_AND_NAME,
        EVERYTHING_SORT_NAME_ASCENDING,
    ] {
        query.extend_from_slice(&value.to_le_bytes());
    }
    for value in search {
        query.extend_from_slice(&value.to_le_bytes());
    }
    query
}

fn parse_everything_reply(bytes: &[u8]) -> Result<Vec<FileEntry>, EverythingError> {
    if bytes.len() < LIST2_HEADER_SIZE {
        return Err(EverythingError::InvalidReply);
    }
    let item_count = read_u32(bytes, 4)? as usize;
    let request_flags = read_u32(bytes, 12)?;
    if request_flags != EVERYTHING_REQUEST_FULL_PATH_AND_NAME {
        return Err(EverythingError::InvalidReply);
    }
    let item_table_end = item_count
        .checked_mul(ITEM2_SIZE)
        .and_then(|size| LIST2_HEADER_SIZE.checked_add(size))
        .filter(|end| *end <= bytes.len())
        .ok_or(EverythingError::InvalidReply)?;

    let mut entries = Vec::with_capacity(item_count.min(RESULT_LIMIT));
    let mut seen = HashSet::new();
    for index in 0..item_count.min(RESULT_LIMIT) {
        let item_offset = LIST2_HEADER_SIZE + index * ITEM2_SIZE;
        let data_offset = read_u32(bytes, item_offset + size_of::<u32>())? as usize;
        if data_offset < item_table_end {
            return Err(EverythingError::InvalidReply);
        }
        let path_text = read_utf16_field(bytes, data_offset)?;
        if path_text.is_empty() {
            continue;
        }
        let path = PathBuf::from(path_text);
        if !seen.insert(path.clone()) {
            continue;
        }
        let name = path
            .file_name()
            .filter(|name| !name.is_empty())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        entries.push(FileEntry { name, path });
    }
    Ok(entries)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, EverythingError> {
    let end = offset
        .checked_add(size_of::<u32>())
        .filter(|end| *end <= bytes.len())
        .ok_or(EverythingError::InvalidReply)?;
    let value: [u8; 4] = bytes[offset..end]
        .try_into()
        .map_err(|_| EverythingError::InvalidReply)?;
    Ok(u32::from_le_bytes(value))
}

fn read_utf16_field(bytes: &[u8], offset: usize) -> Result<String, EverythingError> {
    let character_count = read_u32(bytes, offset)? as usize;
    let text_start = offset
        .checked_add(size_of::<u32>())
        .ok_or(EverythingError::InvalidReply)?;
    let text_bytes = character_count
        .checked_mul(size_of::<u16>())
        .ok_or(EverythingError::InvalidReply)?;
    let text_end = text_start
        .checked_add(text_bytes)
        .ok_or(EverythingError::InvalidReply)?;
    let terminator_end = text_end
        .checked_add(size_of::<u16>())
        .filter(|end| *end <= bytes.len())
        .ok_or(EverythingError::InvalidReply)?;
    if bytes[text_end..terminator_end] != [0, 0] {
        return Err(EverythingError::InvalidReply);
    }
    let utf16: Vec<u16> = bytes[text_start..text_end]
        .chunks_exact(size_of::<u16>())
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    Ok(String::from_utf16_lossy(&utf16))
}

pub(crate) fn everything_available() -> bool {
    let class = wide(EVERYTHING_WINDOW_CLASS);
    !unsafe { FindWindowW(class.as_ptr(), null()) }.is_null()
}

#[cfg(test)]
mod tests {
    use super::{
        EVERYTHING_REQUEST_FULL_PATH_AND_NAME, EverythingClient, EverythingError, FileEntry,
        LIST2_HEADER_SIZE, RESULT_LIMIT, build_everything_query, file_query,
        parse_everything_reply, read_u32,
    };
    use std::{mem::size_of, path::PathBuf};

    #[test]
    fn recognizes_only_explicit_file_search_prefixes() {
        assert_eq!(file_query("f"), Some(""));
        assert_eq!(file_query("f 报告"), Some("报告"));
        assert_eq!(file_query("FILE QuickSpark"), Some("QuickSpark"));
        assert_eq!(file_query("foo"), None);
        assert_eq!(file_query("flow launcher"), None);
    }

    #[test]
    fn builds_a_unicode_everything_query2_request() {
        let query = build_everything_query(42, 99, "报告", RESULT_LIMIT as u32);
        assert_eq!(read_u32(&query, 0), Ok(42));
        assert_eq!(read_u32(&query, 4), Ok(99));
        assert_eq!(read_u32(&query, 16), Ok(RESULT_LIMIT as u32));
        assert_eq!(
            read_u32(&query, 20),
            Ok(EVERYTHING_REQUEST_FULL_PATH_AND_NAME)
        );
        let text: Vec<u16> = query[7 * size_of::<u32>()..]
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        assert_eq!(text, ['报' as u16, '告' as u16, 0]);
    }

    #[test]
    fn parses_unicode_full_paths_from_everything_reply() {
        let paths = [r"C:\Users\Alice\报告.pdf", r"D:\Projects\QuickSpark"];
        let reply = reply_with_paths(&paths);
        assert_eq!(
            parse_everything_reply(&reply),
            Ok(vec![
                FileEntry {
                    name: "报告.pdf".to_owned(),
                    path: PathBuf::from(paths[0]),
                },
                FileEntry {
                    name: "QuickSpark".to_owned(),
                    path: PathBuf::from(paths[1]),
                },
            ])
        );
    }

    #[test]
    fn rejects_truncated_or_inconsistent_everything_replies() {
        assert_eq!(
            parse_everything_reply(&[0; LIST2_HEADER_SIZE - 1]),
            Err(EverythingError::InvalidReply)
        );
        let mut reply = reply_with_paths(&[r"C:\valid.txt"]);
        reply.pop();
        assert_eq!(
            parse_everything_reply(&reply),
            Err(EverythingError::InvalidReply)
        );
    }

    #[test]
    #[ignore = "requires a running Everything 1.4.1 instance"]
    fn live_everything_query_reaches_the_running_index() {
        let mut client = EverythingClient::new().expect("reply window should be created");
        let results = client
            .query("Everything.exe", 5)
            .expect("Everything IPC query should complete");
        assert!(results.iter().all(|entry| entry.path.is_absolute()));
        assert!(!results.is_empty());
    }

    fn reply_with_paths(paths: &[&str]) -> Vec<u8> {
        let item_table_size = paths.len() * 2 * size_of::<u32>();
        let mut reply = Vec::new();
        for value in [
            paths.len() as u32,
            paths.len() as u32,
            0,
            EVERYTHING_REQUEST_FULL_PATH_AND_NAME,
            1,
        ] {
            reply.extend_from_slice(&value.to_le_bytes());
        }

        let mut data_offset = LIST2_HEADER_SIZE + item_table_size;
        let encoded: Vec<Vec<u16>> = paths
            .iter()
            .map(|path| path.encode_utf16().collect())
            .collect();
        for path in &encoded {
            reply.extend_from_slice(&0u32.to_le_bytes());
            reply.extend_from_slice(&(data_offset as u32).to_le_bytes());
            data_offset += size_of::<u32>() + (path.len() + 1) * size_of::<u16>();
        }
        for path in encoded {
            reply.extend_from_slice(&(path.len() as u32).to_le_bytes());
            for character in path {
                reply.extend_from_slice(&character.to_le_bytes());
            }
            reply.extend_from_slice(&0u16.to_le_bytes());
        }
        reply
    }
}
