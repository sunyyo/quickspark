use std::path::PathBuf;

use crate::model::{Action, Query, ResultItem};

use super::{Provider, ProviderContext, ProviderOutput};

pub(crate) struct DriveProvider;

impl Provider for DriveProvider {
    fn id(&self) -> &'static str {
        "drive"
    }

    fn query(&self, query: &Query, _context: &ProviderContext<'_>) -> ProviderOutput {
        let input = query.raw().as_bytes();
        if !matches!(input, [letter, b':'] | [letter, b':', b'\\' | b'/'] if letter.is_ascii_alphabetic())
        {
            return ProviderOutput::empty();
        }

        // A bare "D:" is drive-relative on Windows; always open the root.
        let root = format!("{}:\\", input[0].to_ascii_uppercase() as char);
        ProviderOutput {
            results: vec![ResultItem {
                provider_id: self.id(),
                title: format!("打开磁盘：{root}"),
                score: 100_000,
                actions: vec![
                    Action::OpenPath {
                        path: PathBuf::from(&root),
                    },
                    Action::CopyText { text: root },
                ],
            }],
            exclusive: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::storage::Settings;

    #[test]
    fn drive_queries_open_absolute_roots_without_stealing_other_queries() {
        let usage = HashMap::new();
        let pinned = HashSet::new();
        let settings = Settings::default();
        let context = ProviderContext {
            usage: &usage,
            now: 0,
            settings: &settings,
            pinned: &pinned,
            light_theme: false,
        };

        for letter in b'A'..=b'Z' {
            let root = format!("{}:\\", letter as char);
            for input in [
                format!("{}:", letter as char),
                format!(" {}: ", letter.to_ascii_lowercase() as char),
                root.clone(),
                format!("{}:/", letter as char),
            ] {
                let output = DriveProvider.query(&Query::new(&input), &context);
                assert!(output.exclusive, "{input}");
                assert_eq!(output.results.len(), 1);
                assert_eq!(
                    output.results[0].actions,
                    vec![
                        Action::OpenPath {
                            path: PathBuf::from(&root)
                        },
                        Action::CopyText { text: root.clone() },
                    ]
                );
            }
        }

        for input in [
            "",
            "d",
            "1:",
            "盘:",
            "dd:",
            "d :",
            "d：",
            "d:folder",
            "D:\\folder",
            "https://example.com",
            "f d:",
            "= 1+2",
        ] {
            let output = DriveProvider.query(&Query::new(input), &context);
            assert!(!output.exclusive, "{input}");
            assert!(output.results.is_empty(), "{input}");
        }
    }
}
