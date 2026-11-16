use super::list::selected_row;
use super::msg::{Cmd, Job, ReadJob};
use super::toast::say;
use super::{OpenWith, State};
use crate::clipboard::CopyError;
use crate::domain::GithubRef;
use crate::github::GhError;

/// `o`: the target is fixed at the key press, so a refresh before the job
/// runs can not change what opens.
pub(super) fn open_key(state: &mut State) -> Vec<Cmd> {
    let Some(target) = selected_row(state).map(GithubRef::from) else {
        say(state, "Nothing to open.".to_string());
        return Vec::new();
    };
    match state.context.open_with {
        OpenWith::Browser => {
            say(state, format!("Opening {target} on GitHub"));
            vec![Cmd::Job(Job::Read(ReadJob::Open(target)))]
        }
        OpenWith::Clipboard => {
            say(state, format!("Copying the {target} URL"));
            vec![Cmd::Job(Job::Read(ReadJob::FetchPageUrl(target)))]
        }
    }
}

pub(super) fn opened(state: &mut State, target: GithubRef, result: Result<(), GhError>) -> Vec<Cmd> {
    if let Err(e) = result {
        say(state, format!("Open {target} failed: {e}"));
    }
    Vec::new()
}

pub(super) fn page_url(state: &mut State, target: GithubRef, result: Result<String, GhError>) -> Vec<Cmd> {
    match result {
        Ok(text) => vec![Cmd::Copy { target, text }],
        Err(e) => {
            say(state, format!("Copy {target} URL failed: {e}"));
            Vec::new()
        }
    }
}

pub(super) fn copied(state: &mut State, target: GithubRef, result: Result<String, CopyError>) -> Vec<Cmd> {
    match result {
        Ok(url) => say(state, format!("Copied {url}")),
        Err(e) => say(state, format!("Copy {target} URL failed: {e}")),
    }
    Vec::new()
}
