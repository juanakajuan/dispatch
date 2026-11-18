//! A PR's checks and merge state, from the `statusCheckRollup`,
//! `mergeable`, and `mergeStateStatus` fields of `gh` JSON.

use crate::domain::{Checks, Mergeability};

/// `mergeable` is MERGEABLE, CONFLICTING, or UNKNOWN. `mergeStateStatus` is
/// CLEAN, `HAS_HOOKS`, DIRTY, BEHIND, BLOCKED, UNSTABLE, DRAFT, or UNKNOWN. A
/// value GitHub adds later reads as `Unknown`, so dispatch waits rather than
/// merges.
pub(super) fn mergeability(mergeable: &str, status: &str) -> Mergeability {
    match (mergeable, status) {
        ("CONFLICTING", _) | (_, "DIRTY") => Mergeability::Conflict,
        (_, "CLEAN" | "HAS_HOOKS") => Mergeability::Clean,
        (_, "BEHIND") => Mergeability::Behind,
        (_, "BLOCKED") => Mergeability::Blocked,
        (_, "UNSTABLE") => Mergeability::Unstable,
        _ => Mergeability::Unknown,
    }
}

/// Any failure -> Fail, else any not completed -> Pending, else Pass.
/// Empty -> `Checks::None`.
pub(super) fn rollup(items: &[WireCheck]) -> Checks {
    let outcomes: Vec<Checks> = items
        .iter()
        .filter_map(|c| match c {
            WireCheck::CheckRun { status, conclusion } => Some(if status != "COMPLETED" {
                Checks::Pending
            } else if matches!(
                conclusion.as_str(),
                "FAILURE" | "TIMED_OUT" | "CANCELLED" | "STARTUP_FAILURE" | "ACTION_REQUIRED"
            ) {
                Checks::Fail
            } else {
                Checks::Pass
            }),
            WireCheck::StatusContext { state } => Some(match state.as_str() {
                "FAILURE" | "ERROR" => Checks::Fail,
                "SUCCESS" => Checks::Pass,
                _ => Checks::Pending,
            }),
            WireCheck::Unknown => None,
        })
        .collect();
    if outcomes.is_empty() {
        Checks::None
    } else if outcomes.contains(&Checks::Fail) {
        Checks::Fail
    } else if outcomes.contains(&Checks::Pending) {
        Checks::Pending
    } else {
        Checks::Pass
    }
}

/// Real output mixes two shapes, told apart by `__typename`:
/// `CheckRun` {status: "`COMPLETED"|"IN_PROGRESS`"|.., conclusion: "SUCCESS"|"FAILURE"|""}
/// `StatusContext` {state: "SUCCESS"|"PENDING"|"FAILURE"|"ERROR"}.
#[derive(serde::Deserialize)]
#[serde(tag = "__typename")]
pub(super) enum WireCheck {
    CheckRun {
        #[serde(default)]
        status: String,
        #[serde(default)]
        conclusion: String,
    },
    StatusContext {
        #[serde(default)]
        state: String,
    },
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(status: &str, conclusion: &str) -> WireCheck {
        WireCheck::CheckRun { status: status.to_string(), conclusion: conclusion.to_string() }
    }

    fn status(state: &str) -> WireCheck {
        WireCheck::StatusContext { state: state.to_string() }
    }

    #[test]
    fn rollup_failure_beats_pending_beats_pass() {
        assert_eq!(rollup(&[]), Checks::None);
        assert_eq!(rollup(&[check("COMPLETED", "SUCCESS"), check("COMPLETED", "SKIPPED")]), Checks::Pass);
        assert_eq!(rollup(&[check("IN_PROGRESS", ""), check("COMPLETED", "SUCCESS")]), Checks::Pending);
        assert_eq!(rollup(&[check("IN_PROGRESS", ""), check("COMPLETED", "FAILURE")]), Checks::Fail);
        assert_eq!(rollup(&[status("PENDING"), status("SUCCESS")]), Checks::Pending);
        assert_eq!(rollup(&[status("ERROR"), check("COMPLETED", "SUCCESS")]), Checks::Fail);
        assert_eq!(rollup(&[WireCheck::Unknown]), Checks::None);
    }

    #[test]
    fn mergeability_reads_both_fields_and_waits_on_anything_new() {
        assert_eq!(mergeability("MERGEABLE", "CLEAN"), Mergeability::Clean);
        assert_eq!(mergeability("MERGEABLE", "HAS_HOOKS"), Mergeability::Clean);
        assert_eq!(mergeability("CONFLICTING", "BLOCKED"), Mergeability::Conflict, "a conflict wins");
        assert_eq!(mergeability("UNKNOWN", "DIRTY"), Mergeability::Conflict);
        assert_eq!(mergeability("MERGEABLE", "BEHIND"), Mergeability::Behind);
        assert_eq!(mergeability("MERGEABLE", "BLOCKED"), Mergeability::Blocked);
        assert_eq!(mergeability("MERGEABLE", "UNSTABLE"), Mergeability::Unstable);
        assert_eq!(mergeability("UNKNOWN", "UNKNOWN"), Mergeability::Unknown);
        assert_eq!(mergeability("MERGEABLE", "DRAFT"), Mergeability::Unknown);
        assert_eq!(mergeability("MERGEABLE", "SOMETHING_NEW"), Mergeability::Unknown);
    }
}
