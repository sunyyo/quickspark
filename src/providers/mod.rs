pub(crate) mod ai;
pub(crate) mod applications;
pub(crate) mod calculator;
pub(crate) mod commands;
pub(crate) mod converter;
pub(crate) mod drive;
pub(crate) mod file_search;
pub(crate) mod help;
pub(crate) mod settings;
pub(crate) mod system;
pub(crate) mod url;
pub(crate) mod web_search;
pub(crate) mod window_switcher;

use std::collections::{HashMap, HashSet};

use crate::{
    model::{Query, ResultItem, Usage},
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
}

pub(crate) trait Provider: Send + Sync {
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
