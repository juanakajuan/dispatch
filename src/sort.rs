//! The order of each tab's rows. Pure. Each tab has its own sort enum, so a
//! mode that does not apply to a tab, such as most comments on PRs, can not be
//! set there.

use crate::domain::{Checks, Issue, IssueNumber, Mergeability, PrState, PullRequest};
use crate::session::SessionState;
use std::cmp::Ordering;

/// A tab's sort modes in the order `S` cycles through them.
pub trait Cycle: Copy + PartialEq + 'static {
    const ALL: &'static [Self];

    fn label(self) -> &'static str;

    fn next(self) -> Self {
        let at = Self::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IssueSort {
    /// Needs you, then running, then the rest; newest first in each group.
    #[default]
    Attention,
    Newest,
    Oldest,
    Updated,
    Number,
    Comments,
}

impl Cycle for IssueSort {
    const ALL: &'static [Self] =
        &[Self::Attention, Self::Newest, Self::Oldest, Self::Updated, Self::Number, Self::Comments];

    fn label(self) -> &'static str {
        match self {
            Self::Attention => "attention",
            Self::Newest => "newest",
            Self::Oldest => "oldest",
            Self::Updated => "recently updated",
            Self::Number => "number",
            Self::Comments => "most comments",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrSort {
    /// Open with a problem, then open, draft, merged, closed; newest first in
    /// each group.
    #[default]
    Attention,
    Newest,
    Oldest,
    Updated,
    Number,
}

impl Cycle for PrSort {
    const ALL: &'static [Self] = &[Self::Attention, Self::Newest, Self::Oldest, Self::Updated, Self::Number];

    fn label(self) -> &'static str {
        match self {
            Self::Attention => "attention",
            Self::Newest => "newest",
            Self::Oldest => "oldest",
            Self::Updated => "recently updated",
            Self::Number => "number",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SessionSort {
    /// Needs you, then running, then exited; by number in each group.
    #[default]
    Attention,
    Number,
}

impl Cycle for SessionSort {
    const ALL: &'static [Self] = &[Self::Attention, Self::Number];

    fn label(self) -> &'static str {
        match self {
            Self::Attention => "attention",
            Self::Number => "number",
        }
    }
}

/// Lower comes first. `None` is an issue with no session; the caller passes
/// `Running` for a start in flight.
fn urgency(session: Option<SessionState>) -> u8 {
    match session {
        Some(SessionState::NeedsYou) => 0,
        Some(SessionState::Running) => 1,
        Some(SessionState::Exited) | None => 2,
    }
}

/// Lower comes first. An open PR that can not merge as it is needs someone.
fn pr_urgency(pr: &PullRequest) -> u8 {
    match pr.state {
        PrState::Open if pr.checks == Checks::Fail || pr.mergeability == Mergeability::Conflict => 0,
        PrState::Open => 1,
        PrState::Draft => 2,
        PrState::Merged => 3,
        PrState::Closed => 4,
    }
}

/// Equal keys fall back to the number, in the mode's own direction, so the
/// order never depends on the order `gh` returned.
fn by_number<N: Ord>(ascending: bool, a: N, b: N) -> Ordering {
    if ascending { a.cmp(&b) } else { b.cmp(&a) }
}

pub fn issues(rows: &mut [&Issue], sort: IssueSort, session: impl Fn(IssueNumber) -> Option<SessionState>) {
    rows.sort_by(|a, b| {
        let newest = b.created_at.cmp(&a.created_at);
        let primary = match sort {
            IssueSort::Attention => urgency(session(a.number)).cmp(&urgency(session(b.number))).then(newest),
            IssueSort::Newest => newest,
            IssueSort::Oldest => a.created_at.cmp(&b.created_at),
            IssueSort::Updated => b.updated_at.cmp(&a.updated_at),
            IssueSort::Number => Ordering::Equal,
            IssueSort::Comments => b.comment_count.cmp(&a.comment_count),
        };
        let ascending = matches!(sort, IssueSort::Oldest | IssueSort::Number);
        primary.then_with(|| by_number(ascending, a.number, b.number))
    });
}

pub fn prs(rows: &mut [&PullRequest], sort: PrSort) {
    rows.sort_by(|a, b| {
        let newest = b.created_at.cmp(&a.created_at);
        let primary = match sort {
            PrSort::Attention => pr_urgency(a).cmp(&pr_urgency(b)).then(newest),
            PrSort::Newest => newest,
            PrSort::Oldest => a.created_at.cmp(&b.created_at),
            PrSort::Updated => b.updated_at.cmp(&a.updated_at),
            PrSort::Number => Ordering::Equal,
        };
        let ascending = matches!(sort, PrSort::Oldest | PrSort::Number);
        primary.then_with(|| by_number(ascending, a.number, b.number))
    });
}

pub fn sessions(rows: &mut [IssueNumber], sort: SessionSort, session: impl Fn(IssueNumber) -> Option<SessionState>) {
    match sort {
        SessionSort::Attention => rows.sort_by_key(|n| (urgency(session(*n)), *n)),
        SessionSort::Number => rows.sort(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::PrNumber;
    use crate::testutil::{issue, pr};

    fn at(secs: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(secs).expect("in range")
    }

    /// #1 old, quiet, many comments. #2 newest, updated long ago. #3 middle,
    /// updated last. #4 as old as #1.
    fn sample_issues() -> Vec<Issue> {
        let mk = |n, created, updated, comments| Issue {
            created_at: at(created),
            updated_at: at(updated),
            comment_count: comments,
            ..issue(n, "t", &[])
        };
        vec![mk(3, 200, 900, 1), mk(1, 100, 100, 7), mk(4, 100, 150, 0), mk(2, 300, 300, 2)]
    }

    fn issue_order(sort: IssueSort, session: impl Fn(IssueNumber) -> Option<SessionState>) -> Vec<u32> {
        let items = sample_issues();
        let mut rows: Vec<&Issue> = items.iter().collect();
        issues(&mut rows, sort, session);
        rows.iter().map(|i| i.number.0).collect()
    }

    #[test]
    fn issue_modes() {
        let none = |_| None;
        assert_eq!(issue_order(IssueSort::Newest, none), [2, 3, 4, 1]);
        assert_eq!(issue_order(IssueSort::Oldest, none), [1, 4, 3, 2], "a tie on age goes to the lower number");
        assert_eq!(issue_order(IssueSort::Updated, none), [3, 2, 4, 1]);
        assert_eq!(issue_order(IssueSort::Number, none), [1, 2, 3, 4]);
        assert_eq!(issue_order(IssueSort::Comments, none), [1, 2, 3, 4]);
        assert_eq!(issue_order(IssueSort::Attention, none), [2, 3, 4, 1], "with no sessions it is newest first");
    }

    #[test]
    fn issues_that_need_you_come_first_then_running_then_the_rest_newest_first() {
        let session = |n: IssueNumber| match n.0 {
            1 => Some(SessionState::Running),
            4 => Some(SessionState::NeedsYou),
            3 => Some(SessionState::Running),
            2 => Some(SessionState::Exited),
            _ => None,
        };
        assert_eq!(issue_order(IssueSort::Attention, session), [4, 3, 1, 2]);
    }

    fn sample_prs() -> Vec<PullRequest> {
        let mk = |n, state, checks, mergeability, created, updated| PullRequest {
            state,
            checks,
            mergeability,
            created_at: at(created),
            updated_at: at(updated),
            ..pr(n, "h", &[])
        };
        vec![
            mk(10, PrState::Closed, Checks::None, Mergeability::Unknown, 900, 900),
            mk(11, PrState::Merged, Checks::Pass, Mergeability::Unknown, 800, 950),
            mk(12, PrState::Draft, Checks::Fail, Mergeability::Unknown, 700, 700),
            mk(13, PrState::Open, Checks::Pass, Mergeability::Clean, 100, 600),
            mk(14, PrState::Open, Checks::Pass, Mergeability::Conflict, 200, 200),
            mk(15, PrState::Open, Checks::Pending, Mergeability::Clean, 300, 300),
            mk(16, PrState::Open, Checks::Fail, Mergeability::Unstable, 50, 50),
            mk(17, PrState::Draft, Checks::None, Mergeability::Unknown, 750, 750),
        ]
    }

    fn pr_order(sort: PrSort) -> Vec<u32> {
        let items = sample_prs();
        let mut rows: Vec<&PullRequest> = items.iter().collect();
        prs(&mut rows, sort);
        rows.iter().map(|p| p.number.0).collect()
    }

    #[test]
    fn prs_with_a_problem_come_first_then_open_draft_merged_closed() {
        assert_eq!(pr_order(PrSort::Attention), [14, 16, 15, 13, 17, 12, 11, 10]);
    }

    #[test]
    fn pr_modes() {
        assert_eq!(pr_order(PrSort::Newest), [10, 11, 17, 12, 15, 14, 13, 16]);
        assert_eq!(pr_order(PrSort::Oldest), [16, 13, 14, 15, 12, 17, 11, 10]);
        assert_eq!(pr_order(PrSort::Updated), [11, 10, 17, 12, 13, 15, 14, 16]);
        assert_eq!(pr_order(PrSort::Number), [10, 11, 12, 13, 14, 15, 16, 17]);
        let same = [pr(5, "a", &[]), pr(9, "b", &[])];
        let mut rows: Vec<&PullRequest> = same.iter().collect();
        prs(&mut rows, PrSort::Newest);
        assert_eq!(
            rows.iter().map(|p| p.number).collect::<Vec<_>>(),
            [PrNumber(9), PrNumber(5)],
            "a tie goes to the newer number"
        );
    }

    #[test]
    fn sessions_sort_by_number_as_a_number_and_by_state_in_attention() {
        let state = |n: IssueNumber| match n.0 {
            9 => Some(SessionState::Exited),
            10 => Some(SessionState::Running),
            100 => Some(SessionState::NeedsYou),
            _ => Some(SessionState::Exited),
        };
        let mut rows: Vec<IssueNumber> = [10, 9, 100, 2].map(IssueNumber).to_vec();
        sessions(&mut rows, SessionSort::Number, state);
        assert_eq!(rows, [2, 9, 10, 100].map(IssueNumber));
        sessions(&mut rows, SessionSort::Attention, state);
        assert_eq!(rows, [100, 10, 2, 9].map(IssueNumber));
    }

    #[test]
    fn cycling_visits_every_mode_and_wraps() {
        fn visits<S: Cycle + std::fmt::Debug>(start: S) {
            let mut seen = vec![start];
            let mut s = start.next();
            while s != start {
                seen.push(s);
                s = s.next();
            }
            assert_eq!(seen, S::ALL, "{start:?}");
        }
        visits(IssueSort::default());
        visits(PrSort::default());
        visits(SessionSort::default());
        assert_eq!(IssueSort::default(), IssueSort::Attention);
        assert_eq!(IssueSort::Comments.next(), IssueSort::Attention);
    }
}
