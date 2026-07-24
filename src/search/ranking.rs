use crate::model::Usage;

use super::fuzzy_score;

pub(crate) fn rank_entry(
    name: &str,
    pinyin: &str,
    initials: &str,
    query: &str,
    usage: Usage,
    now: u64,
) -> Option<i32> {
    let name = fuzzy_score(name, query);
    let pinyin = fuzzy_score(pinyin, query).map(|value| value - 3_000);
    let initials = fuzzy_score(initials, query).map(|value| value - 5_000);
    let base = [name, pinyin, initials].into_iter().flatten().max()?;
    let recent = if usage.last > 0 {
        let days = now.saturating_sub(usage.last) / 86_400;
        800i32.saturating_sub(days.min(80) as i32 * 10)
    } else {
        0
    };
    let boost = (usage.count.min(20) as i32 * 70 + recent).min(2_200);
    Some(base + boost)
}
