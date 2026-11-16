pub mod job;
pub mod plan;

use crate::github::GhError;
use crate::proc::ProcError;
use plan::MethodError;
use std::fmt;

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
