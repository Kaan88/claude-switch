//! Grouping, filters and search ranking. No drawing and no clock reads.

use claude_switch_core::Session;
use mdshared_app::fuzzy;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Running,
    Waiting,
    Pinned,
    Unseen,
    Hidden,
    Archived,
}

impl Filter {
    pub const ALL: [Self; 7] =
        [Self::All, Self::Running, Self::Waiting, Self::Pinned, Self::Unseen, Self::Hidden, Self::Archived];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Running => "Running",
            Self::Waiting => "Waiting",
            Self::Pinned => "Pinned",
            Self::Unseen => "Unseen",
            Self::Hidden => "Hidden",
            Self::Archived => "Archived here",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    pub running: bool,
    pub waiting: bool,
    pub pinned: bool,
    pub unseen: bool,
    pub hidden: bool,
}

impl Flags {
    pub fn matches(self, filter: Filter) -> bool {
        match filter {
            Filter::All => true,
            Filter::Running => self.running,
            Filter::Waiting => self.waiting,
            Filter::Pinned => self.pinned,
            Filter::Unseen => self.unseen,
            Filter::Hidden => self.hidden,
            Filter::Archived => false, // Local archive visibility is handled before this filter.
        }
    }
}

pub const DAY: u64 = 86_400_000;
pub const BUCKETS: [&str; 5] = ["Today", "Yesterday", "This week", "Earlier this month", "Older"];

pub fn local_day(ms: u64, offset: i32) -> i64 {
    (ms as i64 + i64::from(offset) * 60_000).div_euclid(DAY as i64)
}

pub struct Calendar {
    offset: i32,
    today: i64,
    week_start: i64,
    month_start: i64,
}

impl Calendar {
    pub fn at(now: u64, offset: i32) -> Self {
        let today = local_day(now, offset);
        let date = mdshared_app::fmt::date((now / 1000) as i64, offset);
        let month_day = date.rsplit_once('-').and_then(|(_, day)| day.parse::<i64>().ok()).unwrap_or(1);
        Self { offset, today, week_start: today - (today + 3).rem_euclid(7), month_start: today + 1 - month_day }
    }
    pub fn bucket(&self, at: u64) -> usize {
        if at == 0 {
            return 4;
        }
        let day = local_day(at, self.offset);
        match self.today - day {
            ..=0 => 0,
            1 => 1,
            _ if day >= self.week_start => 2,
            _ if day >= self.month_start => 3,
            _ => 4,
        }
    }
}

pub fn score(query: &str, name: Option<&str>, s: &Session, folder: &str) -> Option<i32> {
    [name, Some(&s.title), Some(&s.last_prompt), Some(folder), s.cwd.to_str()]
        .into_iter()
        .flatten()
        .flat_map(|text| {
            // The toolkit only scores the first 256 characters; overlap chunks for longer prompts.
            text.char_indices()
                .step_by(192)
                .map(move |(i, _)| fuzzy::score(query, &text[i..]).map(|m| m.score - if i == 0 { 0 } else { 5 }))
        })
        .flatten()
        .max()
}

pub fn rank<'a>(query: &str, candidates: impl IntoIterator<Item = (usize, SessionFields<'a>)>) -> Vec<usize> {
    let mut ranked: Vec<_> = candidates
        .into_iter()
        .filter_map(|(i, f)| {
            let quality = if query.is_empty() { 0 } else { score(query, f.name, f.session, f.folder)? };
            Some((std::cmp::Reverse((quality, f.session.last_active_ms)), i))
        })
        .collect();
    ranked.sort_unstable();
    ranked.into_iter().map(|(_, i)| i).collect()
}

pub struct SessionFields<'a> {
    pub name: Option<&'a str>,
    pub session: &'a Session,
    pub folder: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_midnight_week_month_and_unknown() {
        let bucket = |now, at, offset| Calendar::at(now, offset).bucket(at);
        let thursday = 1_764_244_800_000; // 2025-11-27 12:00 UTC
        assert_eq!(bucket(thursday, thursday, 0), 0);
        assert_eq!(bucket(thursday, thursday - DAY, 0), 1);
        assert_eq!(bucket(thursday, thursday - 2 * DAY, 0), 2);
        assert_eq!(bucket(thursday, thursday - 7 * DAY, 0), 3);
        assert_eq!(bucket(thursday, thursday - 30 * DAY, 0), 4);
        assert_eq!(bucket(thursday, 0, 0), 4);
        let midnight = thursday / DAY * DAY;
        assert_eq!(bucket(midnight, midnight - 10 * 60_000, 0), 1);
        assert_eq!(bucket(midnight, midnight - 10 * 60_000, 60), 0);
        let monday = midnight + 4 * DAY;
        assert_eq!(bucket(monday, monday - 2 * DAY, 0), 4);
    }

    #[test]
    fn better_match_then_newer_and_all_search_fields() {
        let mut list = crate::demo::scan(&crate::demo::config()).sessions;
        list[0].title = "A loose a-b-c".into();
        list[1].title = "ABC older".into();
        list[2].last_prompt = "Unique needle from user".into();
        list[3].last_prompt = format!("{}needle in the end", "z".repeat(472));
        let candidates = || {
            list.iter().take(4).enumerate().map(|(i, session)| {
                (
                    i,
                    SessionFields {
                        session,
                        folder: if i == 2 { "xyz-folder" } else { "" },
                        name: (i == 3).then_some("ABC renamed"),
                    },
                )
            })
        };
        assert_eq!(rank("ABC", candidates())[..2], [1, 3]);
        assert_eq!(rank("unique needle", candidates()), [2]);
        assert_eq!(rank("xyz-folder", candidates()), [2]);
        assert_eq!(rank("renamed", candidates()), [3]);
        assert_eq!(rank("needle in the end", candidates()), [3]);
        assert_eq!(rank("", candidates()), [0, 1, 2, 3]);
    }
}
