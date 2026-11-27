mod ai;
pub(crate) mod applications;
mod calculator;
mod commands;
mod converter;
mod drive;
mod file_search;
mod help;
mod settings;
mod system;
mod url;
pub(crate) mod web_search;
mod window_switcher;

use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, MutexGuard, PoisonError},
};

use crate::{
    model::{Action, Query, ResultItem, Usage},
    storage::Settings,
};

pub(crate) use ai::{AiEvent, AiService};
pub(crate) use applications::{ApplicationProvider, IndexError};
pub(crate) use calculator::CalculatorProvider;
pub(crate) use commands::CommandProvider;
pub(crate) use converter::ConverterProvider;
pub(crate) use drive::DriveProvider;
pub(crate) use file_search::FileSearchProvider;
pub(crate) use help::HelpProvider;
pub(crate) use settings::SettingsProvider;
pub(crate) use system::SystemProvider;
pub(crate) use url::UrlProvider;
pub(crate) use web_search::WebSearchProvider;
pub(crate) use window_switcher::WindowSwitcherProvider;

pub(crate) struct ProviderContext<'a> {
    pub(crate) usage: &'a HashMap<String, Usage>,
    pub(crate) now: u64,
    pub(crate) settings: &'a Settings,
    pub(crate) pinned: &'a HashSet<String>,
    pub(crate) light_theme: bool,
}

pub(crate) struct ProviderOutput {
    pub(crate) results: Vec<ResultItem>,
    pub(crate) exclusive: bool,
}

impl ProviderOutput {
    pub(crate) fn empty() -> Self {
        Self {
            results: Vec::new(),
            exclusive: false,
        }
    }

    pub(super) fn exclusive(results: Vec<ResultItem>) -> Self {
        Self {
            results,
            exclusive: true,
        }
    }

    pub(super) fn exclusive_item(
        provider_id: &'static str,
        title: impl Into<String>,
        score: i32,
        actions: Vec<Action>,
    ) -> Self {
        Self::exclusive(vec![ResultItem {
            provider_id,
            title: title.into(),
            score,
            actions,
        }])
    }
}

pub(super) fn split_first_term(input: &str) -> (&str, &str) {
    let input = input.trim();
    input
        .split_once(char::is_whitespace)
        .map_or((input, ""), |(head, tail)| (head, tail.trim()))
}

pub(super) fn strip_keyword_prefix<'a>(input: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    let (head, tail) = split_first_term(input);
    prefixes
        .iter()
        .any(|prefix| head.eq_ignore_ascii_case(prefix))
        .then_some(tail)
}

pub(super) fn strip_prefix_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

pub(super) fn truncate_chars(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let mut output = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        output.push('…');
    }
    output
}

pub(super) fn matches_all_terms(fields: &[&str], terms: &str) -> bool {
    if terms.is_empty() {
        return true;
    }
    let searchable = fields.join(" ").to_lowercase();
    terms
        .to_lowercase()
        .split_whitespace()
        .all(|term| searchable.contains(term))
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) trait Provider {
    fn id(&self) -> &'static str;
    fn query(&self, query: &Query, context: &ProviderContext<'_>) -> ProviderOutput;
}

pub(crate) struct ProviderRegistry {
    providers: Vec<Box<dyn Provider>>,
}

impl ProviderRegistry {
    pub(crate) fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub(crate) fn register<P>(&mut self, provider: P)
    where
        P: Provider + 'static,
    {
        self.providers
            .retain(|current| current.id() != provider.id());
        self.providers.push(Box::new(provider));
    }

    pub(crate) fn query(
        &self,
        query: &Query,
        context: &ProviderContext<'_>,
        limit: usize,
    ) -> Vec<ResultItem> {
        let mut regular = Vec::new();
        for provider in &self.providers {
            let mut output = provider.query(query, context);
            if output.exclusive {
                sort_and_truncate(&mut output.results, limit, context.pinned);
                return output.results;
            }
            regular.extend(output.results);
        }
        sort_and_truncate(&mut regular, limit, context.pinned);
        regular
    }
}

fn sort_and_truncate(results: &mut Vec<ResultItem>, limit: usize, pinned: &HashSet<String>) {
    for result in results.iter_mut() {
        if result
            .primary_action()
            .and_then(|action| action.favorite_key())
            .is_some_and(|key| pinned.contains(&key))
        {
            result.score = result.score.saturating_add(1_000_000);
        }
    }
    results.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.provider_id.cmp(right.provider_id))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
    });
    results.truncate(limit);
}
