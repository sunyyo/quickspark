use super::{
    alt_shortcut_index, app_cell, catch_index_panic, hotkey_candidates, try_with_state,
    try_with_state_mut, wheel_selection_delta,
};
use crate::{
    model::{Action, Query, Usage},
    providers::{
        ApplicationProvider, CalculatorProvider, ProviderContext, ProviderRegistry,
        WebSearchProvider,
        applications::{
            Entry, EntryTarget, entry_score, is_blocked_tool, is_noise_entry, is_system_setting,
            is_uninstaller, is_user_blocked, parse_start_apps,
        },
        web_search::{parse_web_searches, url_encode},
    },
    search::{fuzzy_score, search_keys},
    storage::{HotkeyChoice, Settings, load_usage_from, save_usage_to},
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn checked_state_access_rejects_reentrant_aliases_without_panicking() {
    let state = RefCell::new(1usize);

    let nested_read = try_with_state_mut(&state, |value| {
        *value += 1;
        try_with_state(&state, |nested| *nested)
    });
    assert_eq!(nested_read, Some(None));

    let nested_write = try_with_state(&state, |_| {
        try_with_state_mut(&state, |nested| *nested += 1)
    });
    assert_eq!(nested_write, Some(None));

    assert_eq!(try_with_state(&state, |value| *value), Some(2));
    assert_eq!(try_with_state_mut(&state, |value| *value += 1), Some(()));
    assert_eq!(try_with_state(&state, |value| *value), Some(3));
}

#[test]
fn null_window_has_no_application_state() {
    assert!(unsafe { app_cell(std::ptr::null_mut()) }.is_none());
}
use windows_sys::Win32::UI::WindowsAndMessaging::{WM_KEYDOWN, WM_SYSKEYDOWN};

#[test]
fn exact_and_substring_rank_above_sparse_match() {
    assert!(
        fuzzy_score("visual studio code", "studio") > fuzzy_score("system settings tool", "studio")
    );
}

#[test]
fn rejects_out_of_order_characters() {
    assert!(fuzzy_score("notepad", "npa").is_some());
    assert!(fuzzy_score("notepad", "zxy").is_none());
}

#[test]
fn chinese_names_generate_full_pinyin_and_initials() {
    assert_eq!(search_keys("微信"), ("weixin".into(), "wx".into()));
    assert_eq!(
        search_keys("网易云音乐"),
        ("wangyiyunyinyue".into(), "wyyyy".into())
    );
}

#[test]
fn mixed_chinese_and_ascii_names_generate_search_keys() {
    assert_eq!(search_keys("腾讯QQ"), ("tengxunqq".into(), "txqq".into()));
}

#[test]
fn web_search_prefixes_build_encoded_urls() {
    assert_eq!(url_encode("天气 today"), "%E5%A4%A9%E6%B0%94%20today");
    let default_searches = parse_web_searches("ss 天气").unwrap();
    assert_eq!(default_searches.len(), 3);
    assert_eq!(default_searches[0].label, "使用百度搜索“天气”");
    assert_eq!(default_searches[1].label, "使用必应搜索“天气”");
    assert_eq!(default_searches[2].label, "使用Google搜索“天气”");
    assert!(
        default_searches[0]
            .url
            .starts_with("https://www.baidu.com/")
    );
    assert!(default_searches[1].url.starts_with("https://www.bing.com/"));
    assert!(
        default_searches[2]
            .url
            .starts_with("https://www.google.com/")
    );
    assert!(
        parse_web_searches("bd Rust 语言").unwrap()[0]
            .url
            .starts_with("https://www.baidu.com/")
    );
    assert!(
        parse_web_searches("BING weather").unwrap()[0]
            .url
            .starts_with("https://www.bing.com/")
    );
    assert!(
        parse_web_searches("gg OpenAI").unwrap()[0]
            .url
            .starts_with("https://www.google.com/")
    );
    assert!(parse_web_searches("ss   ").is_none());
    assert!(parse_web_searches("unknown query").is_none());
}

#[test]
fn provider_registry_returns_exclusive_web_results() {
    let mut registry = ProviderRegistry::new();
    registry.register(ApplicationProvider::empty());
    registry.register(WebSearchProvider);
    let usage = HashMap::new();
    let settings = Settings::default();
    let pinned = HashSet::new();
    let context = ProviderContext {
        usage: &usage,
        now: 100,
        settings: &settings,
        pinned: &pinned,
        light_theme: false,
    };
    let results = registry.query(&Query::new("ss 天气"), &context, 8);
    assert_eq!(results.len(), 3);
    assert!(
        results
            .iter()
            .all(|result| result.provider_id == "web-search")
    );
    assert!(matches!(
        results[0].primary_action(),
        Some(Action::OpenUrl { url }) if url.starts_with("https://www.baidu.com/")
    ));
}

#[test]
fn provider_registry_returns_copyable_calculator_result() {
    let mut registry = ProviderRegistry::new();
    registry.register(ApplicationProvider::empty());
    registry.register(WebSearchProvider);
    registry.register(CalculatorProvider);
    let usage = HashMap::new();
    let settings = Settings::default();
    let pinned = HashSet::new();
    let context = ProviderContext {
        usage: &usage,
        now: 100,
        settings: &settings,
        pinned: &pinned,
        light_theme: false,
    };
    let results = registry.query(&Query::new("= 12 * (8 + 3)"), &context, 8);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].provider_id, "calculator");
    assert_eq!(results[0].title, "= 132");
    assert!(matches!(
        results[0].primary_action(),
        Some(Action::CopyText { text }) if text == "132"
    ));
}

#[test]
fn application_results_expose_context_actions() {
    let provider = ApplicationProvider::from_entries(vec![Entry {
        name: "Example App".into(),
        target: EntryTarget::File(PathBuf::from(r"C:\Apps\Example.exe")),
        key: "example app".into(),
        pinyin: "example app".into(),
        initials: "example app".into(),
    }]);
    let mut registry = ProviderRegistry::new();
    registry.register(provider);
    let usage = HashMap::new();
    let settings = Settings::default();
    let pinned = HashSet::new();
    let context = ProviderContext {
        usage: &usage,
        now: 100,
        settings: &settings,
        pinned: &pinned,
        light_theme: false,
    };
    let results = registry.query(&Query::new("example app"), &context, 8);
    assert_eq!(results.len(), 1);
    assert!(matches!(
        results[0].actions.as_slice(),
        [
            Action::LaunchApplication { .. },
            Action::RunAsAdministrator { .. },
            Action::OpenContainingFolder { .. },
            Action::CopyText { .. }
        ]
    ));
}

#[test]
fn pinned_application_ranks_above_other_matching_results() {
    let entries = ["Alpha App", "Beta App"]
        .into_iter()
        .map(|name| Entry {
            name: name.to_owned(),
            target: EntryTarget::File(PathBuf::from(format!(r"C:\Apps\{name}.exe"))),
            key: name.to_lowercase(),
            pinyin: name.to_lowercase(),
            initials: name.to_lowercase(),
        })
        .collect();
    let mut registry = ProviderRegistry::new();
    registry.register(ApplicationProvider::from_entries(entries));
    let usage = HashMap::new();
    let settings = Settings::default();
    let pinned = HashSet::from(["app:beta app".to_owned()]);
    let context = ProviderContext {
        usage: &usage,
        now: 100,
        settings: &settings,
        pinned: &pinned,
        light_theme: false,
    };
    let results = registry.query(&Query::new("app"), &context, 8);
    assert_eq!(results[0].title, "Beta App");
}

#[test]
fn start_apps_parser_keeps_only_packaged_app_ids() {
    let parsed = parse_start_apps(
        "Calculator\tMicrosoft.WindowsCalculator_8wekyb3d8bbwe!App\n7-Zip\t{GUID}\\7-Zip\\7zFM.exe\n",
    );
    assert_eq!(
        parsed,
        vec![(
            "Calculator".to_owned(),
            "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".to_owned()
        )]
    );
}

#[test]
fn usage_file_round_trips_and_skips_invalid_rows() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("quickspark-test-{}-{unique}", std::process::id()));
    let path = dir.join("usage.tsv");
    let mut expected = HashMap::new();
    expected.insert(
        "obsidian".into(),
        Usage {
            count: 7,
            last: 123,
        },
    );
    assert!(save_usage_to(&path, &expected));
    let loaded = load_usage_from(&path);
    assert_eq!(loaded["obsidian"].count, 7);
    assert_eq!(loaded["obsidian"].last, 123);

    fs::write(&path, "broken row\nvalid\t2\t9\ninvalid\tnope\t4\n").unwrap();
    let recovered = load_usage_from(&path);
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered["valid"].count, 2);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn index_panics_are_reported_as_failures() {
    assert!(catch_index_panic(|| panic!("simulated index failure")).is_err());
}

#[test]
fn detects_common_uninstaller_names() {
    assert!(is_uninstaller("卸载微信"));
    assert!(is_uninstaller("Remove / 移除 My App"));
    assert!(is_uninstaller("Uninstall Visual Studio Code"));
    assert!(is_uninstaller("unins000"));
    assert!(!is_uninstaller("Visual Studio Code"));
}

#[test]
fn blocks_developer_tools_and_environments() {
    for name in [
        "PowerShell",
        "Windows PowerShell (x86)",
        "Windows PowerShell ISE",
        "cmd",
        "命令提示符",
        "Git Bash",
        "Git CMD",
        "Git GUI",
        "Python 3.13",
        "IDLE (Python 3.13)",
        "Anaconda Prompt",
        "Visual Studio Code",
        "IntelliJ IDEA",
        "Node.js",
        "Rust",
        "Docker Desktop",
        "Ubuntu",
    ] {
        assert!(is_blocked_tool(name), "{name} should be blocked");
    }
    assert!(!is_blocked_tool("PowerToys"));
    assert!(!is_blocked_tool("Telegram"));
    assert!(!is_blocked_tool("Google Chrome"));
}

#[test]
fn blocks_system_settings_and_administration_shortcuts() {
    for name in [
        "Settings",
        "控制面板",
        "Windows Tools",
        "计算机管理",
        "Disk Management",
        "服务",
        "Registry Editor",
        "事件查看器",
        "Task Scheduler",
        "系统配置",
        "Local Security Policy",
        "ODBC Data Sources (64-bit)",
        "性能监视器",
    ] {
        assert!(is_system_setting(name), "{name} should be blocked");
    }
    assert!(!is_system_setting("文件资源管理器"));
    assert!(!is_system_setting("Task Manager"));
    assert!(!is_system_setting("Calculator"));
}

#[test]
fn maps_alt_number_shortcuts_to_result_positions() {
    assert_eq!(alt_shortcut_index(WM_SYSKEYDOWN, 0x31), Some(0));
    assert_eq!(alt_shortcut_index(WM_SYSKEYDOWN, 0x38), Some(7));
    assert_eq!(alt_shortcut_index(WM_SYSKEYDOWN, 0x39), None);
    assert_eq!(alt_shortcut_index(WM_KEYDOWN, 0x31), None);
}

#[test]
fn rotates_through_every_hotkey_candidate_once() {
    assert_eq!(
        hotkey_candidates(HotkeyChoice::AltSpace),
        [
            HotkeyChoice::AltSpace,
            HotkeyChoice::ControlSpace,
            HotkeyChoice::AltQ,
            HotkeyChoice::ControlAltSpace,
        ]
    );
    assert_eq!(
        hotkey_candidates(HotkeyChoice::AltQ),
        [
            HotkeyChoice::AltQ,
            HotkeyChoice::ControlAltSpace,
            HotkeyChoice::AltSpace,
            HotkeyChoice::ControlSpace,
        ]
    );
}

#[test]
fn maps_mouse_wheel_direction_and_notches_to_selection_moves() {
    let wparam = |delta: i16| (u16::from_ne_bytes(delta.to_ne_bytes()) as usize) << 16;
    assert_eq!(wheel_selection_delta(wparam(120)), -1);
    assert_eq!(wheel_selection_delta(wparam(-120)), 1);
    assert_eq!(wheel_selection_delta(wparam(240)), -2);
    assert_eq!(wheel_selection_delta(wparam(-240)), 2);
    assert_eq!(wheel_selection_delta(0), 0);
}

#[test]
fn ranking_prefers_exact_prefix_and_dense_matches() {
    assert!(fuzzy_score("obsidian", "obs") > fuzzy_score("my obs tools", "obs"));
    assert!(fuzzy_score("flow launcher", "flow") > fuzzy_score("workflow", "flow"));
    assert!(fuzzy_score("flow launcher", "loe").is_none());
    assert!(fuzzy_score("file explorer", "fe").is_none());
}

#[test]
fn usage_cannot_override_a_relevance_tier() {
    let exact = Entry {
        name: "Obsidian".into(),
        key: "obsidian".into(),
        pinyin: "obsidian".into(),
        initials: "obsidian".into(),
        target: EntryTarget::File(PathBuf::new()),
    };
    let prefix = Entry {
        name: "Obsidian Helper".into(),
        key: "obsidian helper".into(),
        pinyin: "obsidianhelper".into(),
        initials: "obsidianhelper".into(),
        target: EntryTarget::File(PathBuf::new()),
    };
    let exact_score = entry_score(&exact, "obsidian", Usage::default(), 100).unwrap();
    let frequent_score = entry_score(
        &prefix,
        "obsidian",
        Usage {
            count: 999,
            last: 100,
        },
        100,
    )
    .unwrap();
    assert!(exact_score > frequent_score);
    assert!(entry_score(&exact, "", Usage::default(), 100).is_some());
    let saturated = entry_score(
        &exact,
        "obsidian",
        Usage {
            count: u32::MAX,
            last: u64::MAX,
        },
        100,
    );
    assert!(saturated.is_some());
}

#[test]
fn filters_sdk_noise_paths_and_user_patterns() {
    assert!(is_noise_entry(
        "Application Verifier (X64)",
        Path::new(r"C:\ProgramData\Microsoft\Windows\Start Menu")
    ));
    assert!(is_noise_entry(
        "Some Utility",
        Path::new(r"C:\Program Files\Windows Kits\10\bin")
    ));
    assert!(is_user_blocked("Example Helper", &["helper".into()]));
    assert!(!is_user_blocked("Example Player", &["helper".into()]));
}
