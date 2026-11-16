pub mod job;
pub mod plan;

use crate::domain::PrNumber;
use crate::github::GhError;
use crate::proc::ProcError;
use plan::MethodError;
use std::fmt;

/// Merge `pr`, with what the first `m` told the user it would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeAsk {
    pub pr: PrNumber,
    pub consent: Consent,
}

/// If the PR has a problem, a running session on its branch fixes it with no
/// consent. A new session fixes it, and a draft is marked ready, only when
/// the first `m` said so. Otherwise one more `m` must agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    MergeOnly,
    MarkReadyThenMerge,
    StartFixSession,
}

/// Why the second `m` failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeError {
    Gh(GhError),
    Method(MethodError),
    /// `git config --get` of the merge method setting failed.
    Setting(ProcError),
    /// The setting holds `raw`, which names no merge method.
    BadSetting(String),
}

impl fmt::Display for MergeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MergeError::Gh(e) => e.fmt(f),
            MergeError::Method(e) => e.fmt(f),
            MergeError::Setting(e) => e.fmt(f),
            MergeError::BadSetting(raw) => write!(f, "{} is {raw:?}. Use merge, squash, or rebase.", job::SETTING),
        }
    }
}

impl From<GhError> for MergeError {
    fn from(e: GhError) -> MergeError {
        MergeError::Gh(e)
    }
}
