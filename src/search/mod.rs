mod matcher;
mod ranking;

pub(crate) use matcher::{fuzzy_score, search_keys};
pub(crate) use ranking::rank_entry;
