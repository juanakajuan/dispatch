//! UI state and the reducer. `update` is the only writer of `State`; it does
//! no I/O and returns `Cmd`s for the shell to run. Every key, click, tick, and
//! worker result is a `Msg`, so each behaviour is a unit test:
//! "given this state, this msg -> this state and these cmds".

use crate::config::Config;
use crate::domain::{Issue, IssueNumber, Mode, PrNumber, PullRequest, SkillChoice};
use crate::session::{Session, Sessions, StartRequest, WindowId};
use crossterm::event::{KeyEvent, MouseEvent};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Issues,
    Prs,
    Sessions,
}

/// A list row's identity. Selection stores keys, never indices, so a refresh
/// that reorders or filters rows keeps the same item selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKey {
    Issue(IssueNumber),
    Pr(PrNumber),
    Session(IssueNumber),
}

/// Remote data plus its load status. `items` keeps the last good data when a
/// refresh fails, so a network error does not blank the list.
pub struct Feed<T> {
    pub items: Vec<T>,
    pub status: FeedStatus,
}

pub enum FeedStatus {
    Loading,
    Fresh,
    Failed(String),
}

/// Where keys go. One enum, so "dialog open while typing a filter" can not
/// be represented.
pub enum Focus {
    List,
    Filter,
    Dialog(Box<StartDialog>),
}

pub struct StartDialog {
    pub issue: IssueNumber,
    pub title: String,
    pub branch: tui_textarea::TextArea<'static>,
    /// Index into `choices`; 0 is "automatic".
    pub skill: usize,
    pub choices: Vec<SkillChoice>,
    pub skill_reason: Option<String>,
    pub mode: Mode,
    pub prompt: tui_textarea::TextArea<'static>,
    pub field: DialogField,
    /// Inline validation error (bad branch name). The dialog stays open.
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogField {
    Branch,
    Skill,
    Mode,
    Prompt,
}

/// One flag per job kind. The tick only schedules a job whose flag is clear,
/// so a slow `gh` never stacks up a second call.
#[derive(Default)]
pub struct Inflight {
    pub issues: bool,
    pub prs: bool,
    pub sessions: bool,
    pub tail: bool,
}

pub struct Due {
    pub github: Instant,
    pub sessions: Instant,
    pub tail: Instant,
}

pub struct Tail {
    pub window: WindowId,
    pub lines: Vec<String>,
}

/// Clickable regions from the last frame. Written by the shell after each
/// draw (`ui::draw` returns it); read by `update` on mouse events.
#[derive(Default)]
pub struct HitMap {
    pub targets: Vec<(ratatui::layout::Rect, HitTarget)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    Tab(Tab),
    Row(RowKey),
}

pub struct State {
    pub tab: Tab,
    pub issues: Feed<Issue>,
    pub prs: Feed<PullRequest>,
    /// Last observation from `Workspace::observe`. Replaced whole, never patched.
    pub sessions: Sessions,
    /// Issues with a `Job::Start` in flight. The only session-ish state the
    /// app owns; it covers the gap before the next observation shows the window.
    pub starting: BTreeSet<IssueNumber>,
    pub selected: Selected,
    pub filter: String,
    pub focus: Focus,
    pub tail: Option<Tail>,
    pub toast: Option<(String, Instant)>,
    pub inflight: Inflight,
    pub due: Due,
    pub hits: HitMap,
    pub last_click: Option<(HitTarget, Instant)>,
}

#[derive(Default)]
pub struct Selected {
    pub issue: Option<IssueNumber>,
    pub pr: Option<PrNumber>,
    pub session: Option<IssueNumber>,
}

pub enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent, Instant),
    Tick(Instant),
    Issues(Result<Vec<Issue>, String>),
    Prs(Result<Vec<PullRequest>, String>),
    Sessions(Result<Sessions, String>),
    Tail { window: WindowId, result: Result<Vec<String>, String> },
    Started { issue: IssueNumber, result: Result<Session, String> },
    Stopped { issue: IssueNumber, result: Result<(), String> },
    Resumed { issue: IssueNumber, result: Result<(), String> },
    Attached(Result<(), String>),
}

/// Work for the shell.
pub enum Cmd {
    Job(Job),
    /// Runs on the main thread: `AttachPlan::Suspend` must own the terminal.
    Attach(WindowId),
    Quit,
}

/// Runs on a worker thread and comes back as exactly one `Msg`.
pub enum Job {
    FetchIssues,
    FetchPrs,
    ObserveSessions,
    Tail(WindowId),
    Start(StartRequest),
    Stop(Session),
    Resume(Session),
}

pub const SESSIONS_EVERY: Duration = Duration::from_secs(2);
pub const TAIL_EVERY: Duration = Duration::from_secs(1);
pub const TAIL_LINES: u16 = 60;
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

pub fn init(now: Instant) -> (State, Vec<Cmd>) {
    // TODO: State with Feeds Loading, inflight all set, due = now + intervals;
    //       cmds = [FetchIssues, FetchPrs, ObserveSessions].
    todo!()
}

/// The reducer.
pub fn update(state: &mut State, msg: Msg, config: &Config) -> Vec<Cmd> {
    // TODO: match (focus, msg):
    //
    // Dialog + Key:
    //   Esc -> focus = List.  Tab/BackTab -> next/prev field.
    //   Skill field: Left/Right cycles choices. Mode field: Left/Right toggles.
    //   Branch/Prompt: forward to TextArea::input.
    //   Ctrl-Enter / Ctrl-S (and Enter on Branch/Skill/Mode) -> confirm:
    //     BranchName::parse(branch text) Err -> dialog.error, stay.
    //     Ok -> starting.insert(issue); tab = Sessions; selected.session = issue;
    //           focus = List; toast "Starting #N"; [Job(Start(req))].
    //
    // Filter + Key: chars edit `filter`; Enter -> List; Esc -> clear + List.
    //
    // List + Key:
    //   1/2/3 tabs; j/k/Up/Down move within visible_rows; r -> fetch all not inflight;
    //   / -> Filter; Esc -> clear filter; q / Ctrl-C -> [Quit].
    //   s/Enter on Issues: if sessions has issue -> goto Sessions tab, select it;
    //                      elif starting has it -> toast; else open_dialog(issue).
    //   Enter on Prs: goto issue if pr.closes non-empty.
    //   a: session in view (current_session) ->
    //        state Running | NeedsYou -> [Attach(id)];
    //        state Exited -> [Job(Resume(session))], toast "Resuming #N with claude --continue";
    //        the next `a` attaches. (The dead pane's last screen is already in the
    //        detail pane, so attaching to it adds nothing.)
    //   x: session with live window -> [Job(Stop(session))].
    //
    // Mouse: Down(Left) -> hit test; Tab -> set tab; Row -> select; same Row again
    //   within DOUBLE_CLICK on Issues -> as `s`. ScrollUp/Down -> move.
    //
    // Tick(now): expire toast; for each due job whose flag is clear, set the flag,
    //   push the cmd, and set the next due time. Tail only when current_session has
    //   a window (alive or dead).
    //
    // Results: clear the matching inflight flag; Ok replaces data; Err keeps data and
    //   sets FeedStatus::Failed / toast. Tail result for a window other than the one in
    //   view is dropped. Started/Stopped/Resumed: remove from `starting`, toast on Err,
    //   and push ObserveSessions (if not inflight) so the list catches up at once.
    todo!()
}

// ---- pure view helpers, shared by update and ui ----------------------------

/// Rows of the current tab after the filter. The single definition of
/// "what is in the list"; ui renders it and update moves through it.
pub fn visible_rows(state: &State) -> Vec<RowKey> {
    // TODO: Issues -> issues filtered by rules::matches_filter(["#N", title, labels..]).
    //       Prs    -> prs filtered by ["#N", title, head].
    //       Sessions -> sessions by issue number, newest (highest) first, filtered by
    //                   ["#N", issue title if known, branch].
    todo!()
}

/// The stored key if it is visible, else the first visible row.
pub fn selected_row(state: &State) -> Option<RowKey> {
    todo!()
}

/// The session the `a`/`x` keys act on: the selected session, the selected
/// issue's session, or the session of the issue the selected PR closes.
pub fn current_session(state: &State) -> Option<&Session> {
    todo!()
}

/// A PR whose head branch equals the session branch.
pub fn pr_for_session<'a>(state: &'a State, session: &Session) -> Option<&'a PullRequest> {
    todo!()
}

#[cfg(test)]
mod tests {
    // TODO: s on an issue with a session goes to it and opens no dialog.
    // TODO: confirm with an invalid branch keeps the dialog open with an error and
    //       emits no Cmd.
    // TODO: s twice: second press while `starting` has the issue emits no Start.
    // TODO: Tick while inflight.issues is set does not emit a second FetchIssues.
    // TODO: Issues(Err) keeps the old items.
    // TODO: double-click inside DOUBLE_CLICK opens the dialog; outside only selects.
    // TODO: filter that hides the selected row selects the first visible row; clearing
    //       the filter selects the original row again.
    // TODO: Tail result for a stale window id is dropped.
}
