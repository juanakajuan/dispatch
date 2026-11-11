//! UI state and the reducer. `update` is the only writer of `State`; it does
//! no I/O and returns `Cmd`s for the shell to run. Every key, click, tick, and
//! worker result is a `Msg`, so each behaviour is a unit test:
//! "given this state, this msg -> this state and these cmds".

use crate::config::Config;
use crate::domain::{BranchName, GithubRef, Issue, IssueNumber, MergeMethod, Mode, PrNumber, PrState, PullRequest, Split};
use crate::merge::MergeOutcome;
use crate::rules::{self, MergePlan, MergeProblem};
use crate::session::{Session, SessionState, Sessions, StartFrom, StartOutcome, StartRequest, WindowId};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
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

/// The page `o` opens for a row. A session row opens its issue. Exhaustive,
/// so a new row kind does not compile until it says what `o` opens.
impl From<RowKey> for GithubRef {
    fn from(key: RowKey) -> GithubRef {
        match key {
            RowKey::Issue(n) | RowKey::Session(n) => GithubRef::Issue(n),
            RowKey::Pr(n) => GithubRef::Pr(n),
        }
    }
}

/// Remote data plus its load status. `items` keeps the last good data when a
/// refresh fails, so a network error does not blank the list.
pub struct Feed<T> {
    pub items: Vec<T>,
    pub status: FeedStatus,
}

/// `at` is when the response arrived, not when the request was sent.
pub enum FeedStatus {
    Loading,
    Fresh { at: Instant },
    Failed { error: String, at: Instant },
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
    pub mode: Mode,
    pub prompt: tui_textarea::TextArea<'static>,
    pub field: DialogField,
    /// Inline validation error (bad branch name). The dialog stays open.
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogField {
    Branch,
    Mode,
    Prompt,
}

const FIELDS: [DialogField; 3] = [DialogField::Branch, DialogField::Mode, DialogField::Prompt];

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
    /// Set by an event that likely changed GitHub data. A tick fetches once
    /// no fetch is in flight and `GITHUB_COOLDOWN` has passed since
    /// `github_sent`, so a burst of events makes one fetch.
    pub github_soon: bool,
    /// When the last tick sent a GitHub fetch.
    pub github_sent: Instant,
    pub sessions: Instant,
    /// `None` means "as soon as possible": set when the selection changes.
    pub tail: Option<Instant>,
}

pub struct Tail {
    pub window: WindowId,
    pub lines: Vec<String>,
}

/// A message for the key bar. `update` has no clock for key presses, so the
/// next tick stamps `shown_at`, and a later tick removes the toast.
pub struct Toast {
    pub text: String,
    pub shown_at: Option<Instant>,
}

/// Clickable regions and pane geometry from the last frame. Written by the
/// shell after each draw (`ui::draw` returns it); read by `update` on mouse
/// events and resize keys.
#[derive(Default)]
pub struct HitMap {
    pub targets: Vec<(ratatui::layout::Rect, HitTarget)>,
    /// The body the list and detail panes share. Zero before the first draw.
    pub panes: ratatui::layout::Rect,
}

impl HitMap {
    fn at(&self, column: u16, row: u16) -> Option<HitTarget> {
        let pos = ratatui::layout::Position { x: column, y: row };
        self.targets.iter().find(|(rect, _)| rect.contains(pos)).map(|(_, t)| *t)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    Tab(Tab),
    Row(RowKey),
    /// The border between the list and the detail pane.
    Divider,
}

/// Facts about the repo that the view shows, resolved once at startup.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub repo_name: String,
    /// `origin/main`
    pub base: String,
}

pub struct State {
    pub context: Context,
    pub tab: Tab,
    pub issues: Feed<Issue>,
    pub prs: Feed<PullRequest>,
    /// Last observation from `Workspace::observe`. Replaced whole on each
    /// observation. A finished start adds its own session at once, so the row
    /// does not blink out until the next observation.
    pub sessions: Sessions,
    /// Issues with a `Job::Start` in flight. The only session-ish state the
    /// app owns; it covers the gap before the next observation shows the window.
    pub starting: BTreeSet<IssueNumber>,
    pub selected: Selected,
    pub filter: String,
    pub focus: Focus,
    pub tail: Option<Tail>,
    pub toast: Option<Toast>,
    pub inflight: Inflight,
    pub due: Due,
    pub hits: HitMap,
    pub last_click: Option<(HitTarget, Instant)>,
    /// The list | detail border, the same on every tab.
    pub split: Split,
    /// True from a press on the divider to the next release.
    pub dragging_split: bool,
    /// Set by an `m` that waits for one more `m` on the same PR. Any other
    /// key clears it.
    pub armed: Option<Armed>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Armed {
    /// The next `m` checks the PR again, then merges it or has it fixed.
    Merge(MergeAsk),
    /// The check found a problem and no running session is on the PR's
    /// branch. The next `m` starts `req`, a session that fixes it.
    Fix { pr: PrNumber, req: StartRequest },
}

/// Merge `pr`. If it has a problem, a running session on its branch fixes it.
/// Without one, a new session fixes it only when `new_session_ok`, because
/// the first `m` told the user so. Otherwise one more `m` must agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeAsk {
    pub pr: PrNumber,
    pub new_session_ok: bool,
}

/// Who fixes a PR that can not merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fixer {
    /// A running session on the PR's branch. It gets a prompt.
    Agent(IssueNumber),
    /// A new session for this issue, on the PR's branch.
    NewSession(IssueNumber),
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
    /// `at` is stamped by the worker when `gh` returns.
    Issues { result: Result<Vec<Issue>, String>, at: Instant },
    Prs { result: Result<Vec<PullRequest>, String>, at: Instant },
    Sessions(Result<Sessions, String>),
    Tail { window: WindowId, result: Result<Vec<String>, String> },
    Started { issue: IssueNumber, result: Result<StartOutcome, String> },
    Stopped { issue: IssueNumber, result: Result<(), String> },
    Resumed { issue: IssueNumber, result: Result<(), String> },
    Attached(Result<(), String>),
    Opened { target: GithubRef, result: Result<(), String> },
    Sent { issue: IssueNumber, result: Result<(), String> },
    Merged { ask: MergeAsk, result: Result<MergeOutcome, String> },
    /// The terminal got focus back, often after a merge or comment in the browser.
    FocusGained,
}

/// Work for the shell.
pub enum Cmd {
    Job(Job),
    /// Runs on the main thread: `AttachPlan::Suspend` must own the terminal.
    Attach(WindowId),
    Quit,
}

/// Runs on a worker thread and comes back as exactly one `Msg`. `Stop` and
/// `Resume` carry the issue, not the session, because the worker observes
/// again before it acts, so a stale row can not aim at a reused window id.
#[derive(Debug, PartialEq, Eq)]
pub enum Job {
    FetchIssues,
    FetchPrs,
    ObserveSessions,
    Tail(WindowId),
    Start(StartRequest),
    Stop(IssueNumber),
    Resume(IssueNumber),
    /// Open the page in the browser. No inflight flag: two presses may open
    /// two tabs, which is what the user asked for.
    Open(GithubRef),
    /// Type a line into the session's Claude prompt and submit it.
    Send { issue: IssueNumber, text: String },
    /// Check the PR again, then merge it or report what the agent must fix.
    Merge(MergeAsk),
}

pub const SESSIONS_EVERY: Duration = Duration::from_secs(2);
pub const TAIL_EVERY: Duration = Duration::from_secs(1);
pub const TAIL_LINES: u16 = 60;
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const TOAST_FOR: Duration = Duration::from_secs(5);
const GITHUB_COOLDOWN: Duration = Duration::from_secs(5);

pub fn init(now: Instant, config: &Config, context: Context) -> (State, Vec<Cmd>) {
    let state = State {
        context,
        tab: Tab::Issues,
        issues: Feed { items: Vec::new(), status: FeedStatus::Loading },
        prs: Feed { items: Vec::new(), status: FeedStatus::Loading },
        sessions: Sessions::new(),
        starting: BTreeSet::new(),
        selected: Selected::default(),
        filter: String::new(),
        focus: Focus::List,
        tail: None,
        toast: None,
        inflight: Inflight { issues: true, prs: true, sessions: true, tail: false },
        due: Due {
            github: now + config.github_refresh,
            github_soon: false,
            github_sent: now,
            sessions: now + SESSIONS_EVERY,
            tail: None,
        },
        hits: HitMap::default(),
        last_click: None,
        split: config.split,
        dragging_split: false,
        armed: None,
    };
    (state, vec![Cmd::Job(Job::FetchIssues), Cmd::Job(Job::FetchPrs), Cmd::Job(Job::ObserveSessions)])
}

// ---- key table -------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Code(KeyCode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    GoTab(Tab),
    Down,
    Up,
    Start,
    Enter,
    Attach,
    Stop,
    Open,
    Merge,
    Refresh,
    Filter,
    ClearFilter,
    SplitNarrower,
    SplitWider,
    SplitReset,
    Quit,
}

/// When a key bar entry shows. Key handling does not use it: a key always
/// reaches its action, and the action explains when it does not apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Always,
    OnIssues,
    OnPrs,
    OnSessions,
    SessionLive,
    SessionExited,
    /// The selection has an open, non-draft PR.
    Mergeable,
}

pub struct BarEntry {
    pub keys: &'static str,
    pub desc: &'static str,
    pub when: When,
}

pub struct Binding {
    pub keys: &'static [Key],
    pub action: Action,
    pub bar: &'static [BarEntry],
}

const fn bar(keys: &'static str, desc: &'static str, when: When) -> BarEntry {
    BarEntry { keys, desc, when }
}

/// The one table behind both key handling and the key bar.
pub static BINDINGS: &[Binding] = &[
    Binding { keys: &[Key::Char('j'), Key::Code(KeyCode::Down)], action: Action::Down, bar: &[bar("j/k", "move", When::Always)] },
    Binding { keys: &[Key::Char('k'), Key::Code(KeyCode::Up)], action: Action::Up, bar: &[] },
    Binding { keys: &[Key::Char('1')], action: Action::GoTab(Tab::Issues), bar: &[bar("1-3", "tabs", When::Always)] },
    Binding { keys: &[Key::Char('2')], action: Action::GoTab(Tab::Prs), bar: &[] },
    Binding { keys: &[Key::Char('3')], action: Action::GoTab(Tab::Sessions), bar: &[] },
    Binding { keys: &[Key::Char('s')], action: Action::Start, bar: &[bar("s", "start session", When::OnIssues)] },
    Binding {
        keys: &[Key::Code(KeyCode::Enter)],
        action: Action::Enter,
        bar: &[bar("enter", "go to issue", When::OnPrs), bar("enter", "attach", When::OnSessions)],
    },
    Binding {
        keys: &[Key::Char('a')],
        action: Action::Attach,
        bar: &[bar("a", "attach", When::SessionLive), bar("a", "resume", When::SessionExited)],
    },
    Binding { keys: &[Key::Char('x')], action: Action::Stop, bar: &[bar("x", "stop", When::SessionLive)] },
    Binding { keys: &[Key::Char('m')], action: Action::Merge, bar: &[bar("m", "merge", When::Mergeable)] },
    Binding { keys: &[Key::Char('o')], action: Action::Open, bar: &[bar("o", "open on GitHub", When::Always)] },
    Binding { keys: &[Key::Char('r')], action: Action::Refresh, bar: &[bar("r", "refresh", When::Always)] },
    Binding { keys: &[Key::Char('/')], action: Action::Filter, bar: &[bar("/", "filter", When::Always)] },
    Binding { keys: &[Key::Code(KeyCode::Esc)], action: Action::ClearFilter, bar: &[] },
    Binding { keys: &[Key::Char('<')], action: Action::SplitNarrower, bar: &[] },
    Binding { keys: &[Key::Char('>')], action: Action::SplitWider, bar: &[] },
    Binding { keys: &[Key::Char('=')], action: Action::SplitReset, bar: &[] },
    Binding { keys: &[Key::Char('q'), Key::Ctrl('c')], action: Action::Quit, bar: &[bar("q", "quit", When::Always)] },
];

/// The key bar for the current context: (keys, description).
pub fn key_bar(state: &State) -> Vec<(&'static str, &'static str)> {
    let session = current_session(state).map(Session::state);
    let shown = |when: When| match when {
        When::Always => true,
        When::OnIssues => state.tab == Tab::Issues,
        When::OnPrs => state.tab == Tab::Prs,
        When::OnSessions => state.tab == Tab::Sessions && session.is_some(),
        When::SessionLive => matches!(session, Some(SessionState::Running | SessionState::NeedsYou)),
        When::SessionExited => session == Some(SessionState::Exited),
        When::Mergeable => merge_target(state).is_ok(),
    };
    BINDINGS.iter().flat_map(|b| b.bar).filter(|e| shown(e.when)).map(|e| (e.keys, e.desc)).collect()
}

fn key_of(ev: &KeyEvent) -> Option<Key> {
    let plain = ev.modifiers.difference(KeyModifiers::SHIFT).is_empty();
    match ev.code {
        KeyCode::Char(c) if ev.modifiers.contains(KeyModifiers::CONTROL) => Some(Key::Ctrl(c.to_ascii_lowercase())),
        KeyCode::Char(c) if plain => Some(Key::Char(c)),
        KeyCode::Char(_) => None,
        code => Some(Key::Code(code)),
    }
}

fn action_for(ev: &KeyEvent) -> Option<Action> {
    let key = key_of(ev)?;
    BINDINGS.iter().find(|b| b.keys.contains(&key)).map(|b| b.action)
}

// ---- the reducer -------------------------------------------------------------

/// The reducer.
pub fn update(state: &mut State, msg: Msg, config: &Config) -> Vec<Cmd> {
    match msg {
        Msg::Key(ev) if ev.kind == KeyEventKind::Release => Vec::new(),
        Msg::Key(ev) => match &state.focus {
            Focus::Dialog(_) => dialog_key(state, &ev),
            Focus::Filter => filter_key(state, &ev),
            Focus::List => {
                let armed = state.armed.take();
                match action_for(&ev) {
                    Some(action) => act(state, action, armed, config),
                    None => Vec::new(),
                }
            }
        },
        Msg::Mouse(ev, now) => on_mouse(state, &ev, now),
        Msg::Tick(now) => tick(state, now, config),
        Msg::Issues { result, at } => {
            state.inflight.issues = false;
            apply_feed(&mut state.issues, result, at);
            Vec::new()
        }
        Msg::Prs { result, at } => {
            state.inflight.prs = false;
            apply_feed(&mut state.prs, result, at);
            Vec::new()
        }
        Msg::Sessions(result) => {
            state.inflight.sessions = false;
            match result {
                Ok(sessions) => {
                    if state.sessions.iter().any(|(n, s)| went_idle(s.state(), sessions.get(n).map(Session::state))) {
                        state.due.github_soon = true;
                    }
                    state.sessions = sessions;
                }
                Err(e) => say(state, format!("sessions: {e}")),
            }
            Vec::new()
        }
        Msg::Tail { window, result } => {
            state.inflight.tail = false;
            let in_view = current_session(state).and_then(|s| s.window.as_ref()).is_some_and(|w| w.id == window);
            if let (true, Ok(lines)) = (in_view, result) {
                state.tail = Some(Tail { window, lines });
            }
            Vec::new()
        }
        Msg::Started { issue, result } => {
            state.starting.remove(&issue);
            match result {
                Ok(outcome) => {
                    let text = match outcome.warnings.first() {
                        Some(w) => format!("Started #{} (warning: {w})", issue.0),
                        None => format!("Started #{}", issue.0),
                    };
                    state.sessions.insert(issue, outcome.session);
                    state.due.github_soon = true;
                    say(state, text);
                }
                Err(e) => say(state, format!("Start #{} failed: {e}", issue.0)),
            }
            observe_now(state)
        }
        Msg::Stopped { issue, result } => {
            state.due.github_soon = true;
            match result {
                Ok(()) => say(state, format!("Stopped #{}", issue.0)),
                Err(e) => say(state, format!("Stop #{} failed: {e}", issue.0)),
            }
            observe_now(state)
        }
        Msg::Resumed { issue, result } => {
            match result {
                Ok(()) => say(state, format!("Resumed #{}. Press a to attach.", issue.0)),
                Err(e) => say(state, format!("Resume #{} failed: {e}", issue.0)),
            }
            observe_now(state)
        }
        Msg::Opened { target, result } => {
            if let Err(e) = result {
                say(state, format!("Open {target} failed: {e}"));
            }
            Vec::new()
        }
        Msg::Sent { issue, result } => {
            match result {
                Ok(()) => say(state, format!("Asked #{}'s agent to fix and merge its PR", issue.0)),
                Err(e) => say(state, format!("Merge request to #{} failed: {e}", issue.0)),
            }
            Vec::new()
        }
        Msg::Merged { ask, result } => merged(state, ask, result),
        Msg::Attached(result) => {
            if let Err(e) = result {
                say(state, format!("attach failed: {e}"));
            }
            observe_now(state)
        }
        Msg::FocusGained => {
            state.due.github_soon = true;
            Vec::new()
        }
    }
}

/// True when a session's Claude stopped working: it waits for you, or it
/// exited. That is usually when it has just opened or pushed a PR.
fn went_idle(before: SessionState, after: Option<SessionState>) -> bool {
    matches!(
        (before, after),
        (SessionState::Running, Some(SessionState::NeedsYou | SessionState::Exited) | None)
            | (SessionState::NeedsYou, Some(SessionState::Exited) | None)
    )
}

fn say(state: &mut State, text: String) {
    state.toast = Some(Toast { text, shown_at: None });
}

fn observe_now(state: &mut State) -> Vec<Cmd> {
    if state.inflight.sessions {
        return Vec::new();
    }
    state.inflight.sessions = true;
    vec![Cmd::Job(Job::ObserveSessions)]
}

fn apply_feed<T>(feed: &mut Feed<T>, result: Result<Vec<T>, String>, at: Instant) {
    match result {
        Ok(items) => {
            feed.items = items;
            feed.status = FeedStatus::Fresh { at };
        }
        Err(error) => feed.status = FeedStatus::Failed { error, at },
    }
}

fn tick(state: &mut State, now: Instant, config: &Config) -> Vec<Cmd> {
    if let Some(toast) = &mut state.toast {
        match toast.shown_at {
            None => toast.shown_at = Some(now),
            Some(at) if now.duration_since(at) >= TOAST_FOR => state.toast = None,
            Some(_) => {}
        }
    }
    let mut cmds = Vec::new();
    let idle = !state.inflight.issues && !state.inflight.prs;
    let soon = state.due.github_soon && idle && now >= state.due.github_sent + GITHUB_COOLDOWN;
    if soon || now >= state.due.github {
        // A fetch already in flight may predate the event, so `soon` waits for it.
        if idle {
            state.due.github_soon = false;
        }
        state.due.github = now + config.github_refresh;
        state.due.github_sent = now;
        refresh_github(state, &mut cmds);
    }
    if now >= state.due.sessions {
        state.due.sessions = now + SESSIONS_EVERY;
        cmds.extend(observe_now(state));
    }
    if !state.inflight.tail && state.due.tail.is_none_or(|at| now >= at) {
        let window = current_session(state).and_then(|s| s.window.as_ref()).map(|w| w.id.clone());
        if let Some(id) = window {
            state.inflight.tail = true;
            state.due.tail = Some(now + TAIL_EVERY);
            cmds.push(Cmd::Job(Job::Tail(id)));
        }
    }
    cmds
}

fn refresh_github(state: &mut State, cmds: &mut Vec<Cmd>) {
    if !state.inflight.issues {
        state.inflight.issues = true;
        cmds.push(Cmd::Job(Job::FetchIssues));
    }
    if !state.inflight.prs {
        state.inflight.prs = true;
        cmds.push(Cmd::Job(Job::FetchPrs));
    }
}

/// `armed` is what a previous `m` armed; only the next `m` uses it.
fn act(state: &mut State, action: Action, armed: Option<Armed>, config: &Config) -> Vec<Cmd> {
    match action {
        Action::GoTab(tab) => {
            state.tab = tab;
            state.due.tail = None;
        }
        Action::Down => move_selection(state, Move::Down),
        Action::Up => move_selection(state, Move::Up),
        Action::Start => return start_key(state),
        Action::Enter => match state.tab {
            Tab::Issues => return start_key(state),
            Tab::Prs => go_to_issue_of_pr(state),
            Tab::Sessions => return attach_key(state),
        },
        Action::Attach => return attach_key(state),
        Action::Stop => {
            let live = current_session(state).filter(|s| matches!(s.state(), SessionState::Running | SessionState::NeedsYou));
            if let Some(issue) = live.map(|s| s.issue) {
                say(state, format!("Stopping #{}", issue.0));
                return vec![Cmd::Job(Job::Stop(issue))];
            }
            say(state, "No running session here.".to_string());
        }
        Action::Open => return open_key(state),
        Action::Merge => return merge_key(state, armed),
        Action::Refresh => {
            let mut cmds = Vec::new();
            refresh_github(state, &mut cmds);
            cmds.extend(observe_now(state));
            return cmds;
        }
        Action::Filter => state.focus = Focus::Filter,
        Action::ClearFilter => state.filter.clear(),
        Action::SplitNarrower => state.split = state.split.narrower(state.hits.panes.width),
        Action::SplitWider => state.split = state.split.wider(state.hits.panes.width),
        Action::SplitReset => state.split = config.split,
        Action::Quit => return vec![Cmd::Quit],
    }
    Vec::new()
}

fn attach_key(state: &mut State) -> Vec<Cmd> {
    let Some(session) = current_session(state) else {
        say(state, "No session for this issue. Press s to start one.".to_string());
        return Vec::new();
    };
    let issue = session.issue;
    match (&session.window, session.state()) {
        (Some(w), SessionState::Running | SessionState::NeedsYou) => vec![Cmd::Attach(w.id.clone())],
        _ => {
            say(state, format!("Resuming #{} with claude --continue", issue.0));
            vec![Cmd::Job(Job::Resume(issue))]
        }
    }
}

/// `o`: the target is fixed at the key press, so a refresh before the job
/// runs can not change what opens.
fn open_key(state: &mut State) -> Vec<Cmd> {
    let Some(target) = selected_row(state).map(GithubRef::from) else {
        say(state, "Nothing to open.".to_string());
        return Vec::new();
    };
    say(state, format!("Opening {target} on GitHub"));
    vec![Cmd::Job(Job::Open(target))]
}

/// The PR `m` acts on: the selected PR, or the PR of the selected issue or
/// session. No session is needed.
fn merge_target(state: &State) -> Result<&PullRequest, String> {
    let (pr, missing) = match selected_row(state).ok_or("Nothing is selected.")? {
        RowKey::Pr(n) => (state.prs.items.iter().find(|p| p.number == n), format!("PR #{} is not in the list.", n.0)),
        RowKey::Issue(n) => (
            state.sessions.get(&n).and_then(|s| pr_for_session(state, s)).or_else(|| lead_pr(state, n)),
            format!("#{} has no PR yet.", n.0),
        ),
        RowKey::Session(n) => (state.sessions.get(&n).and_then(|s| pr_for_session(state, s)), format!("#{} has no PR yet.", n.0)),
    };
    let pr = pr.ok_or(missing)?;
    match pr.state {
        PrState::Open => Ok(pr),
        PrState::Draft => Err(format!("PR #{} is a draft.", pr.number.0)),
        PrState::Merged | PrState::Closed => Err(format!("PR #{} is not open.", pr.number.0)),
    }
}

/// A running session on the PR's branch fixes it. Else a new session does:
/// for the issue of an exited session on that branch, or else for the first
/// issue the PR closes that has no session yet.
fn fixer(state: &State, pr: &PullRequest) -> Result<Fixer, String> {
    if let Some(s) = state.sessions.values().find(|s| s.branch.as_str() == pr.head) {
        return Ok(match s.state() {
            SessionState::Running | SessionState::NeedsYou => Fixer::Agent(s.issue),
            SessionState::Exited => Fixer::NewSession(s.issue),
        });
    }
    let n = pr.number.0;
    if pr.closes.is_empty() {
        return Err(format!("PR #{n} closes no issue, so dispatch can not start a session for it."));
    }
    pr.closes
        .iter()
        .find(|i| !state.sessions.contains_key(i) && !state.starting.contains(i))
        .map(|&i| Fixer::NewSession(i))
        .ok_or_else(|| format!("Each issue that PR #{n} closes already has a session on another branch."))
}

/// `m`: the first press arms, the next press on the same PR acts. Merging
/// is hard to undo and a new session is a big step, so one stray key never
/// does either. The first press says what the PR list expects; the job
/// checks GitHub again before it acts.
fn merge_key(state: &mut State, armed: Option<Armed>) -> Vec<Cmd> {
    let pr = match merge_target(state) {
        Ok(pr) => pr,
        Err(why) => {
            say(state, why);
            return Vec::new();
        }
    };
    let n = pr.number;
    if let Some(Armed::Fix { pr, req }) = armed.clone()
        && pr == n
    {
        return start_fix(state, req);
    }
    let (new_session_ok, expect) = match (rules::merge_plan(pr), fixer(state, pr)) {
        (MergePlan::Handoff(problem), fix) => {
            let sentence = rules::problem_sentence(n, &pr.base, problem);
            match fix {
                Ok(Fixer::Agent(i)) => (false, format!("{sentence} Press m again to ask #{}'s agent to fix and merge it.", i.0)),
                Ok(Fixer::NewSession(i)) => (true, format!("{sentence} Press m again to start a session for #{} that fixes it.", i.0)),
                Err(why) => {
                    say(state, format!("{sentence} {why}"));
                    return Vec::new();
                }
            }
        }
        // A cached refusal still arms, because the list can be stale and the
        // job asks GitHub again.
        (MergePlan::Direct | MergePlan::Refuse(_), _) => (false, format!("Press m again to merge PR #{}", n.0)),
    };
    let ask = MergeAsk { pr: n, new_session_ok };
    if armed != Some(Armed::Merge(ask)) {
        state.armed = Some(Armed::Merge(ask));
        say(state, expect);
        return Vec::new();
    }
    say(state, format!("Checking PR #{}", n.0));
    vec![Cmd::Job(Job::Merge(ask))]
}

fn start_fix(state: &mut State, req: StartRequest) -> Vec<Cmd> {
    let n = req.issue;
    state.starting.insert(n);
    goto_session(state, n);
    vec![Cmd::Job(Job::Start(req))]
}

fn fix_request(issue: IssueNumber, pr: &PullRequest, problem: MergeProblem, method: MergeMethod) -> Result<StartRequest, String> {
    let branch = BranchName::parse(&pr.head).map_err(|e| format!("PR #{} has the branch name {:?}: {e:?}", pr.number.0, pr.head))?;
    Ok(StartRequest {
        issue,
        branch,
        mode: Mode::Implement,
        prompt: rules::fix_session_prompt(pr.number, &pr.base, problem, method),
        from: StartFrom::Origin,
    })
}

fn merged(state: &mut State, ask: MergeAsk, result: Result<MergeOutcome, String>) -> Vec<Cmd> {
    match result {
        Ok(MergeOutcome::Merged(method)) => {
            say(state, format!("Merged PR #{} ({method})", ask.pr.0));
            let mut cmds = Vec::new();
            refresh_github(state, &mut cmds);
            cmds
        }
        Ok(MergeOutcome::Queued) => {
            say(state, format!("PR #{} is in the merge queue", ask.pr.0));
            let mut cmds = Vec::new();
            refresh_github(state, &mut cmds);
            cmds
        }
        Ok(MergeOutcome::Handoff { problem, method, pr }) => handoff(state, ask, &pr, problem, method),
        Ok(MergeOutcome::Refused(why)) => {
            say(state, why);
            Vec::new()
        }
        Err(e) => {
            say(state, format!("Merge PR #{} failed: {e}", ask.pr.0));
            Vec::new()
        }
    }
}

/// The check found `problem` on `pr`, the PR as GitHub showed it now.
fn handoff(state: &mut State, ask: MergeAsk, pr: &PullRequest, problem: MergeProblem, method: MergeMethod) -> Vec<Cmd> {
    let sentence = rules::problem_sentence(pr.number, &pr.base, problem);
    let issue = match fixer(state, pr) {
        Ok(Fixer::Agent(i)) => {
            say(state, format!("{sentence} Asking #{}'s agent to fix it.", i.0));
            return vec![Cmd::Job(Job::Send { issue: i, text: rules::handoff_prompt(pr.number, &pr.base, problem, method) })];
        }
        Ok(Fixer::NewSession(i)) => i,
        Err(why) => {
            say(state, format!("{sentence} {why}"));
            return Vec::new();
        }
    };
    let req = match fix_request(issue, pr, problem, method) {
        Ok(req) => req,
        Err(why) => {
            say(state, format!("{sentence} {why}"));
            return Vec::new();
        }
    };
    if ask.new_session_ok {
        say(state, format!("{sentence} Starting a session for #{} to fix it.", issue.0));
        return start_fix(state, req);
    }
    state.armed = Some(Armed::Fix { pr: pr.number, req });
    say(state, format!("{sentence} Press m again to start a session for #{} that fixes it.", issue.0));
    Vec::new()
}

/// `s` or Enter on an issue, or a double-click on one.
fn start_key(state: &mut State) -> Vec<Cmd> {
    let Some(RowKey::Issue(n)) = selected_row(state).filter(|_| state.tab == Tab::Issues) else {
        return Vec::new();
    };
    start_issue(state, n);
    Vec::new()
}

fn start_issue(state: &mut State, n: IssueNumber) {
    if state.sessions.contains_key(&n) {
        goto_session(state, n);
    } else if state.starting.contains(&n) {
        say(state, format!("#{} is already starting", n.0));
    } else if let Some(issue) = state.issues.items.iter().find(|i| i.number == n) {
        state.focus = Focus::Dialog(Box::new(open_dialog(issue)));
    }
}

fn goto_session(state: &mut State, n: IssueNumber) {
    state.tab = Tab::Sessions;
    state.selected.session = Some(n);
    state.filter.clear();
    state.due.tail = None;
}

fn go_to_issue_of_pr(state: &mut State) {
    let Some(RowKey::Pr(n)) = selected_row(state) else { return };
    let closes = state.prs.items.iter().find(|p| p.number == n).and_then(|p| p.closes.first().copied());
    match closes {
        Some(issue) => {
            state.tab = Tab::Issues;
            state.selected.issue = Some(issue);
            state.filter.clear();
        }
        None => say(state, format!("PR #{} closes no issue.", n.0)),
    }
}

#[derive(Clone, Copy)]
enum Move {
    Down,
    Up,
}

fn move_selection(state: &mut State, step: Move) {
    let rows = visible_rows(state);
    if rows.is_empty() {
        return;
    }
    let at = selected_row(state).and_then(|k| rows.iter().position(|r| *r == k)).unwrap_or(0);
    let to = match step {
        Move::Down => (at + 1).min(rows.len() - 1),
        Move::Up => at.saturating_sub(1),
    };
    select(state, rows[to]);
}

fn select(state: &mut State, key: RowKey) {
    match key {
        RowKey::Issue(n) => state.selected.issue = Some(n),
        RowKey::Pr(n) => state.selected.pr = Some(n),
        RowKey::Session(n) => state.selected.session = Some(n),
    }
    state.due.tail = None;
}

fn on_mouse(state: &mut State, ev: &MouseEvent, now: Instant) -> Vec<Cmd> {
    // Before the focus gate, so a drag can not stay stuck behind a dialog.
    if matches!(ev.kind, MouseEventKind::Up(_) | MouseEventKind::Down(_)) {
        state.dragging_split = false;
    }
    if !matches!(state.focus, Focus::List) {
        return Vec::new();
    }
    match ev.kind {
        MouseEventKind::ScrollDown => move_selection(state, Move::Down),
        MouseEventKind::ScrollUp => move_selection(state, Move::Up),
        // Follows the pointer even off the divider, so a fast drag keeps hold.
        MouseEventKind::Drag(MouseButton::Left) if state.dragging_split => {
            let panes = state.hits.panes;
            state.split = Split::at_column(panes.width, ev.column.saturating_sub(panes.x));
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(target) = state.hits.at(ev.column, ev.row) else { return Vec::new() };
            let double = state.last_click.is_some_and(|(t, at)| t == target && now.duration_since(at) <= DOUBLE_CLICK);
            state.last_click = if double { None } else { Some((target, now)) };
            match target {
                HitTarget::Tab(tab) => {
                    state.tab = tab;
                    state.due.tail = None;
                }
                HitTarget::Divider => state.dragging_split = true,
                HitTarget::Row(key) => {
                    select(state, key);
                    if let (true, RowKey::Issue(n)) = (double, key) {
                        start_issue(state, n);
                    }
                }
            }
        }
        _ => {}
    }
    Vec::new()
}

fn filter_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    match key_of(ev) {
        Some(Key::Code(KeyCode::Enter)) => state.focus = Focus::List,
        Some(Key::Code(KeyCode::Esc)) => {
            state.filter.clear();
            state.focus = Focus::List;
        }
        Some(Key::Code(KeyCode::Backspace)) => {
            state.filter.pop();
        }
        Some(Key::Ctrl('c')) => return vec![Cmd::Quit],
        Some(Key::Char(c)) => state.filter.push(c),
        _ => {}
    }
    Vec::new()
}

// ---- the start dialog --------------------------------------------------------

fn open_dialog(issue: &Issue) -> StartDialog {
    let defaults = rules::start_defaults(issue);
    let mut branch = tui_textarea::TextArea::new(vec![defaults.branch.as_str().to_string()]);
    branch.move_cursor(tui_textarea::CursorMove::End);
    let prompt = tui_textarea::TextArea::new(defaults.prompt.split('\n').map(str::to_string).collect());
    StartDialog {
        issue: issue.number,
        title: issue.title.clone(),
        branch,
        mode: defaults.mode,
        prompt,
        field: DialogField::Branch,
        error: None,
    }
}

fn dialog_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Dialog(dialog) = &mut state.focus else { return Vec::new() };
    let key = key_of(ev);
    let on_text = matches!(dialog.field, DialogField::Branch | DialogField::Prompt);
    match key {
        Some(Key::Code(KeyCode::Esc)) => state.focus = Focus::List,
        Some(Key::Ctrl('s')) => return confirm_dialog(state),
        Some(Key::Code(KeyCode::Enter)) if dialog.field != DialogField::Prompt => return confirm_dialog(state),
        Some(Key::Code(KeyCode::Enter)) if ev.modifiers.contains(KeyModifiers::CONTROL) => return confirm_dialog(state),
        Some(Key::Code(KeyCode::Tab)) => dialog.field = step_field(dialog.field, 1),
        Some(Key::Code(KeyCode::BackTab)) => dialog.field = step_field(dialog.field, FIELDS.len() - 1),
        Some(Key::Code(KeyCode::Down)) if dialog.field != DialogField::Prompt => dialog.field = step_field(dialog.field, 1),
        Some(Key::Code(KeyCode::Up)) if dialog.field != DialogField::Prompt => dialog.field = step_field(dialog.field, FIELDS.len() - 1),
        Some(Key::Code(KeyCode::Left | KeyCode::Right) | Key::Char(' ')) if dialog.field == DialogField::Mode => {
            dialog.mode = match dialog.mode {
                Mode::PlanFirst => Mode::Implement,
                Mode::Implement => Mode::PlanFirst,
            };
        }
        _ if on_text => {
            let target = if dialog.field == DialogField::Branch { &mut dialog.branch } else { &mut dialog.prompt };
            target.input(*ev);
            dialog.error = None;
        }
        _ => {}
    }
    Vec::new()
}

fn step_field(field: DialogField, by: usize) -> DialogField {
    let at = FIELDS.iter().position(|f| *f == field).unwrap_or(0);
    FIELDS[(at + by) % FIELDS.len()]
}

fn confirm_dialog(state: &mut State) -> Vec<Cmd> {
    let Focus::Dialog(dialog) = &mut state.focus else { return Vec::new() };
    let branch = match BranchName::parse(&dialog.branch.lines().join("")) {
        Ok(b) => b,
        Err(e) => {
            dialog.error = Some(format!("Branch name: {e:?}"));
            dialog.field = DialogField::Branch;
            return Vec::new();
        }
    };
    let prompt = dialog.prompt.lines().join("\n").trim().to_string();
    if prompt.is_empty() {
        dialog.error = Some("The prompt is empty.".to_string());
        dialog.field = DialogField::Prompt;
        return Vec::new();
    }
    let req = StartRequest {
        issue: dialog.issue,
        branch,
        mode: dialog.mode,
        prompt,
        from: StartFrom::Base,
    };
    let n = req.issue;
    state.starting.insert(n);
    state.focus = Focus::List;
    goto_session(state, n);
    say(state, format!("Starting #{}", n.0));
    vec![Cmd::Job(Job::Start(req))]
}

// ---- pure view helpers, shared by update and ui ----------------------------

/// Rows of the current tab after the filter. The single definition of
/// "what is in the list"; ui renders it and update moves through it.
pub fn visible_rows(state: &State) -> Vec<RowKey> {
    let f = state.filter.as_str();
    match state.tab {
        Tab::Issues => state
            .issues
            .items
            .iter()
            .filter(|i| {
                let number = format!("#{}", i.number.0);
                let mut parts = vec![number.as_str(), i.title.as_str()];
                parts.extend(i.labels.iter().map(String::as_str));
                rules::matches_filter(&parts, f)
            })
            .map(|i| RowKey::Issue(i.number))
            .collect(),
        Tab::Prs => state
            .prs
            .items
            .iter()
            .filter(|p| rules::matches_filter(&[&format!("#{}", p.number.0), &p.title, &p.head], f))
            .map(|p| RowKey::Pr(p.number))
            .collect(),
        Tab::Sessions => {
            let numbers: BTreeSet<IssueNumber> = state.sessions.keys().chain(state.starting.iter()).copied().collect();
            numbers
                .into_iter()
                .rev()
                .filter(|n| {
                    let title = state.issues.items.iter().find(|i| i.number == *n).map_or("", |i| i.title.as_str());
                    let branch = state.sessions.get(n).map_or("", |s| s.branch.as_str());
                    rules::matches_filter(&[&format!("#{}", n.0), title, branch], f)
                })
                .map(RowKey::Session)
                .collect()
        }
    }
}

/// The stored key if it is visible, else the first visible row.
pub fn selected_row(state: &State) -> Option<RowKey> {
    let rows = visible_rows(state);
    let stored = match state.tab {
        Tab::Issues => state.selected.issue.map(RowKey::Issue),
        Tab::Prs => state.selected.pr.map(RowKey::Pr),
        Tab::Sessions => state.selected.session.map(RowKey::Session),
    };
    stored.filter(|k| rows.contains(k)).or_else(|| rows.first().copied())
}

/// The session the `a`/`x` keys act on: the selected session, the selected
/// issue's session, or the session of the issue the selected PR closes.
pub fn current_session(state: &State) -> Option<&Session> {
    match selected_row(state)? {
        RowKey::Issue(n) | RowKey::Session(n) => state.sessions.get(&n),
        RowKey::Pr(n) => {
            let pr = state.prs.items.iter().find(|p| p.number == n)?;
            pr.closes
                .iter()
                .find_map(|c| state.sessions.get(c))
                .or_else(|| state.sessions.values().find(|s| s.branch.as_str() == pr.head))
        }
    }
}

/// PRs that name issue `n` in their closing references.
pub fn prs_closing(state: &State, n: IssueNumber) -> impl Iterator<Item = &PullRequest> {
    state.prs.items.iter().filter(move |p| p.closes.contains(&n))
}

/// The one PR an issue row shows: merged beats open beats draft, then the
/// newest number wins. A closed PR never shows.
pub fn lead_pr(state: &State, n: IssueNumber) -> Option<&PullRequest> {
    let rank = |p: &PullRequest| match p.state {
        PrState::Merged => Some(3),
        PrState::Open => Some(2),
        PrState::Draft => Some(1),
        PrState::Closed => None,
    };
    prs_closing(state, n).filter_map(|p| Some(((rank(p)?, p.number), p))).max_by_key(|(k, _)| *k).map(|(_, p)| p)
}

/// A PR whose head branch equals the session branch.
pub fn pr_for_session<'a>(state: &'a State, session: &Session) -> Option<&'a PullRequest> {
    state.prs.items.iter().find(|p| p.head == session.branch.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Mergeability, WorktreePath};
    use crate::session::Window;
    use crate::testutil::{issue, pr};
    use ratatui::layout::Rect;
    use std::path::PathBuf;

    fn key(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn code(c: KeyCode) -> Msg {
        Msg::Key(KeyEvent::new(c, KeyModifiers::NONE))
    }

    fn ctrl(c: char) -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn click(col: u16, row: u16, at: Instant) -> Msg {
        Msg::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: col, row, modifiers: KeyModifiers::NONE }, at)
    }

    fn session(n: u32, window: Option<(bool, bool)>) -> Session {
        Session {
            issue: IssueNumber(n),
            branch: BranchName::parse(&format!("fix/{n}-x")).expect("branch"),
            worktree: WorktreePath::planned(PathBuf::from(format!("/w/orbit-{n}"))),
            window: window.map(|(alive, waiting)| Window { id: WindowId::parse(&format!("@{n}")).expect("id"), alive, waiting }),
        }
    }

    /// Three issues (#3 bug, #2 docs, #1 feature), idle jobs.
    fn app() -> (State, Config, Instant) {
        let now = Instant::now();
        let config = Config::default();
        let (mut state, _) = init(now, &config, Context::default());
        state.issues = Feed {
            items: vec![issue(3, "Retry queue drops jobs", &["bug"]), issue(2, "Docs for plugins", &["docs"]), issue(1, "Add json output", &["feature"])],
            status: FeedStatus::Fresh { at: now },
        };
        state.prs = Feed { items: vec![pr(10, "fix/3-x", &[3]), pr(11, "other", &[])], status: FeedStatus::Fresh { at: now } };
        state.inflight = Inflight::default();
        (state, config, now)
    }

    fn press(state: &mut State, config: &Config, msg: Msg) -> Vec<Cmd> {
        update(state, msg, config)
    }

    fn dialog(state: &State) -> &StartDialog {
        match &state.focus {
            Focus::Dialog(d) => d,
            _ => panic!("no dialog is open"),
        }
    }

    fn starts(cmds: &[Cmd]) -> Vec<&StartRequest> {
        cmds.iter().filter_map(|c| match c { Cmd::Job(Job::Start(r)) => Some(r), _ => None }).collect()
    }

    #[test]
    fn init_asks_for_all_three_reads() {
        let (state, cmds) = init(Instant::now(), &Config::default(), Context::default());
        let jobs: Vec<_> = cmds.iter().map(|c| matches!(c, Cmd::Job(_))).collect();
        assert_eq!(jobs, [true, true, true]);
        assert!(state.inflight.issues && state.inflight.prs && state.inflight.sessions);
    }

    #[test]
    fn s_on_an_issue_opens_the_dialog_with_label_defaults() {
        let (mut state, config, _) = app();
        assert!(press(&mut state, &config, key('s')).is_empty());
        let d = dialog(&state);
        assert_eq!(d.branch.lines(), ["fix/3-retry-queue-drops-jobs"]);
        assert!(d.prompt.lines()[0].starts_with("Implement GitHub issue #3: Retry queue drops jobs"));
    }

    #[test]
    fn s_on_an_issue_with_a_session_goes_to_it_and_opens_no_dialog() {
        let (mut state, config, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        let cmds = press(&mut state, &config, key('s'));
        assert!(cmds.is_empty());
        assert_eq!(state.tab, Tab::Sessions);
        assert_eq!(selected_row(&state), Some(RowKey::Session(IssueNumber(3))));
        assert!(matches!(state.focus, Focus::List));
    }

    #[test]
    fn confirm_with_an_invalid_branch_keeps_the_dialog_open_with_an_error() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('s'));
        press(&mut state, &config, key('x'));
        let Focus::Dialog(d) = &mut state.focus else { panic!("dialog") };
        d.branch.insert_str(" bad name");
        let cmds = press(&mut state, &config, ctrl('s'));
        assert!(cmds.is_empty());
        assert!(dialog(&state).error.is_some());
        assert!(state.starting.is_empty());
    }

    #[test]
    fn confirm_starts_once_and_a_second_s_emits_no_start() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('s'));
        let cmds = press(&mut state, &config, ctrl('s'));
        let reqs = starts(&cmds);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].issue, IssueNumber(3));
        assert_eq!(reqs[0].branch.as_str(), "fix/3-retry-queue-drops-jobs");
        assert!(reqs[0].prompt.starts_with("Implement GitHub issue #3"));
        assert_eq!(state.tab, Tab::Sessions);
        assert_eq!(visible_rows(&state), [RowKey::Session(IssueNumber(3))]);

        press(&mut state, &config, key('1'));
        let again = press(&mut state, &config, key('s'));
        assert!(again.is_empty());
        assert!(matches!(state.focus, Focus::List), "no dialog while starting");
    }

    #[test]
    fn enter_confirms_on_the_branch_field_but_adds_a_newline_in_the_prompt() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('s'));
        for _ in 0..2 {
            press(&mut state, &config, code(KeyCode::Tab));
        }
        assert_eq!(dialog(&state).field, DialogField::Prompt);
        let lines = dialog(&state).prompt.lines().len();
        assert!(press(&mut state, &config, code(KeyCode::Enter)).is_empty());
        assert_eq!(dialog(&state).prompt.lines().len(), lines + 1);
        press(&mut state, &config, code(KeyCode::Tab));
        assert_eq!(dialog(&state).field, DialogField::Branch);
        let cmds = press(&mut state, &config, code(KeyCode::Enter));
        assert_eq!(starts(&cmds).len(), 1);
    }

    #[test]
    fn dialog_mode_toggles_and_reaches_the_request() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('s'));
        press(&mut state, &config, code(KeyCode::BackTab));
        assert_eq!(dialog(&state).field, DialogField::Prompt);
        press(&mut state, &config, code(KeyCode::Tab));
        press(&mut state, &config, code(KeyCode::Tab));
        assert_eq!(dialog(&state).field, DialogField::Mode);
        press(&mut state, &config, code(KeyCode::Right));
        assert_eq!(dialog(&state).mode, Mode::PlanFirst);
        let cmds = press(&mut state, &config, ctrl('s'));
        assert_eq!(starts(&cmds)[0].mode, Mode::PlanFirst);
    }

    #[test]
    fn esc_closes_the_dialog_without_a_command() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('s'));
        assert!(press(&mut state, &config, code(KeyCode::Esc)).is_empty());
        assert!(matches!(state.focus, Focus::List));
        assert!(state.starting.is_empty());
    }

    #[test]
    fn tick_does_not_stack_a_second_fetch_while_one_is_inflight() {
        let (mut state, config, now) = app();
        state.due.github = now;
        state.inflight.issues = true;
        let cmds = press(&mut state, &config, Msg::Tick(now + Duration::from_millis(1)));
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchIssues))));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
        let later = press(&mut state, &config, Msg::Tick(now + Duration::from_millis(2)));
        assert!(later.is_empty(), "due time moved forward");
    }

    #[test]
    fn tick_observes_sessions_every_two_seconds_once() {
        let (mut state, config, now) = app();
        let t = now + SESSIONS_EVERY;
        let cmds = press(&mut state, &config, Msg::Tick(t));
        assert_eq!(cmds.iter().filter(|c| matches!(c, Cmd::Job(Job::ObserveSessions))).count(), 1);
        let cmds = press(&mut state, &config, Msg::Tick(t + Duration::from_millis(100)));
        assert!(cmds.is_empty());
    }

    #[test]
    fn issues_error_keeps_the_old_items_and_stamps_the_failure() {
        let (mut state, config, now) = app();
        state.inflight.issues = true;
        let failed = now + Duration::from_secs(5);
        press(&mut state, &config, Msg::Issues { result: Err("offline".to_string()), at: failed });
        assert_eq!(state.issues.items.len(), 3);
        assert!(matches!(&state.issues.status, FeedStatus::Failed { error, at } if error == "offline" && *at == failed));
        assert!(!state.inflight.issues);
        let arrived = now + Duration::from_secs(9);
        press(&mut state, &config, Msg::Issues { result: Ok(vec![issue(9, "new", &[])]), at: arrived });
        assert_eq!(state.issues.items.len(), 1);
        assert!(matches!(state.issues.status, FeedStatus::Fresh { at } if at == arrived));
    }

    #[test]
    fn new_data_resets_the_refresh_time_on_the_timer_and_after_r() {
        let (mut state, config, _) = app();
        let due = state.due.github;
        let cmds = press(&mut state, &config, Msg::Tick(due));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
        let first = due + Duration::from_secs(2);
        press(&mut state, &config, Msg::Prs { result: Ok(Vec::new()), at: first });
        assert!(matches!(state.prs.status, FeedStatus::Fresh { at } if at == first));

        let cmds = press(&mut state, &config, key('r'));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
        let second = first + Duration::from_secs(30);
        press(&mut state, &config, Msg::Prs { result: Ok(Vec::new()), at: second });
        assert!(matches!(state.prs.status, FeedStatus::Fresh { at } if at == second));
    }

    fn hit_state() -> (State, Config, Instant) {
        let (mut state, config, now) = app();
        state.hits.targets = vec![
            (Rect::new(0, 0, 10, 1), HitTarget::Tab(Tab::Issues)),
            (Rect::new(10, 0, 10, 1), HitTarget::Tab(Tab::Prs)),
            (Rect::new(0, 2, 30, 1), HitTarget::Row(RowKey::Issue(IssueNumber(3)))),
            (Rect::new(0, 3, 30, 1), HitTarget::Row(RowKey::Issue(IssueNumber(2)))),
        ];
        (state, config, now)
    }

    #[test]
    fn click_selects_a_row_and_a_tab() {
        let (mut state, config, now) = hit_state();
        press(&mut state, &config, click(5, 3, now));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
        press(&mut state, &config, click(12, 0, now));
        assert_eq!(state.tab, Tab::Prs);
    }

    #[test]
    fn double_click_inside_the_window_opens_the_dialog_and_outside_only_selects() {
        let (mut state, config, now) = hit_state();
        press(&mut state, &config, click(5, 3, now));
        press(&mut state, &config, click(5, 3, now + DOUBLE_CLICK - Duration::from_millis(1)));
        assert_eq!(dialog(&state).issue, IssueNumber(2));

        let (mut state, config, now) = hit_state();
        press(&mut state, &config, click(5, 3, now));
        press(&mut state, &config, click(5, 3, now + DOUBLE_CLICK + Duration::from_millis(1)));
        assert!(matches!(state.focus, Focus::List));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
    }

    #[test]
    fn wheel_moves_the_selection() {
        let (mut state, config, now) = app();
        let wheel = |kind| Msg::Mouse(MouseEvent { kind, column: 0, row: 0, modifiers: KeyModifiers::NONE }, now);
        press(&mut state, &config, wheel(MouseEventKind::ScrollDown));
        press(&mut state, &config, wheel(MouseEventKind::ScrollDown));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
        press(&mut state, &config, wheel(MouseEventKind::ScrollDown));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))), "clamped at the end");
        press(&mut state, &config, wheel(MouseEventKind::ScrollUp));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
    }

    #[test]
    fn jk_move_and_stay_inside_the_list() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('k'));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(3))));
        press(&mut state, &config, key('j'));
        press(&mut state, &config, code(KeyCode::Down));
        press(&mut state, &config, key('j'));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
    }

    #[test]
    fn a_filter_that_hides_the_selection_selects_the_first_visible_and_clearing_restores_it() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('j'));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
        press(&mut state, &config, key('/'));
        assert!(matches!(state.focus, Focus::Filter));
        for c in "json".chars() {
            press(&mut state, &config, key(c));
        }
        press(&mut state, &config, code(KeyCode::Enter));
        assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(1))]);
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
        press(&mut state, &config, code(KeyCode::Esc));
        assert_eq!(visible_rows(&state).len(), 3);
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
    }

    #[test]
    fn filter_esc_clears_and_backspace_edits() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('/'));
        press(&mut state, &config, key('a'));
        press(&mut state, &config, key('b'));
        press(&mut state, &config, code(KeyCode::Backspace));
        assert_eq!(state.filter, "a");
        press(&mut state, &config, code(KeyCode::Esc));
        assert_eq!((state.filter.as_str(), matches!(state.focus, Focus::List)), ("", true));
    }

    #[test]
    fn stale_tail_is_dropped_and_current_tail_is_kept() {
        let (mut state, config, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        state.inflight.tail = true;
        let stale = WindowId::parse("@99").expect("id");
        press(&mut state, &config, Msg::Tail { window: stale, result: Ok(vec!["old".to_string()]) });
        assert!(state.tail.is_none());
        assert!(!state.inflight.tail);
        let current = WindowId::parse("@3").expect("id");
        press(&mut state, &config, Msg::Tail { window: current, result: Ok(vec!["new".to_string()]) });
        assert_eq!(state.tail.as_ref().map(|t| t.lines.clone()), Some(vec!["new".to_string()]));
    }

    #[test]
    fn tick_asks_for_the_tail_of_the_session_in_view_only() {
        let (mut state, config, now) = app();
        let cmds = press(&mut state, &config, Msg::Tick(now));
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Tail(_)))));
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        let cmds = press(&mut state, &config, Msg::Tick(now));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Tail(id)) if id.as_str() == "@3")), "dead panes have a tail too");
    }

    #[test]
    fn a_attaches_a_live_session_and_resumes_an_exited_one() {
        let (mut state, config, _) = app();
        let cmds = press(&mut state, &config, key('a'));
        assert!(cmds.is_empty());
        assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("Press s")));

        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        let cmds = press(&mut state, &config, key('a'));
        assert!(matches!(cmds.as_slice(), [Cmd::Attach(id)] if id.as_str() == "@3"));

        state.sessions.insert(IssueNumber(3), session(3, Some((true, true))));
        assert!(matches!(press(&mut state, &config, key('a')).as_slice(), [Cmd::Attach(_)]), "needs-you attaches too");

        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        let cmds = press(&mut state, &config, key('a'));
        assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Resume(n))] if *n == IssueNumber(3)));
        assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("claude --continue")));
    }

    #[test]
    fn x_stops_only_a_live_session_by_issue_number() {
        let (mut state, config, _) = app();
        assert!(press(&mut state, &config, key('x')).is_empty());
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        assert!(press(&mut state, &config, key('x')).is_empty(), "exited has nothing to stop");
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        let cmds = press(&mut state, &config, key('x'));
        assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Stop(n))] if *n == IssueNumber(3)));
    }

    #[test]
    fn pr_enter_goes_to_the_closed_issue_and_pr_rows_show_the_session() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('2'));
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        assert_eq!(current_session(&state).map(|s| s.issue), Some(IssueNumber(3)), "via closes");
        press(&mut state, &config, code(KeyCode::Enter));
        assert_eq!(state.tab, Tab::Issues);
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(3))));

        press(&mut state, &config, key('2'));
        press(&mut state, &config, key('j'));
        assert_eq!(selected_row(&state), Some(RowKey::Pr(PrNumber(11))));
        press(&mut state, &config, code(KeyCode::Enter));
        assert_eq!(state.tab, Tab::Prs, "a PR without closes stays");
    }

    #[test]
    fn lead_pr_prefers_merged_then_open_then_draft_and_skips_closed() {
        let (mut state, _, _) = app();
        let with = |n: u32, s: PrState| PullRequest { state: s, ..pr(n, &format!("b{n}"), &[7]) };
        state.prs.items = vec![with(20, PrState::Closed)];
        assert!(lead_pr(&state, IssueNumber(7)).is_none(), "a closed PR never shows");
        state.prs.items.push(with(21, PrState::Draft));
        assert_eq!(lead_pr(&state, IssueNumber(7)).map(|p| p.number), Some(PrNumber(21)));
        state.prs.items.push(with(22, PrState::Open));
        state.prs.items.push(with(23, PrState::Open));
        assert_eq!(lead_pr(&state, IssueNumber(7)).map(|p| p.number), Some(PrNumber(23)), "newest open wins a tie");
        state.prs.items.push(with(19, PrState::Merged));
        assert_eq!(lead_pr(&state, IssueNumber(7)).map(|p| p.number), Some(PrNumber(19)));
        assert!(lead_pr(&state, IssueNumber(8)).is_none());
    }

    #[test]
    fn session_pr_is_found_by_head_branch() {
        let (state, _, _) = app();
        let s = session(3, None);
        assert_eq!(pr_for_session(&state, &s).map(|p| p.number), Some(PrNumber(10)));
        assert!(pr_for_session(&state, &session(4, None)).is_none());
    }

    #[test]
    fn sessions_tab_lists_sessions_and_starting_issues_newest_first() {
        let (mut state, config, _) = app();
        state.sessions.insert(IssueNumber(1), session(1, None));
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        state.starting.insert(IssueNumber(2));
        press(&mut state, &config, key('3'));
        assert_eq!(visible_rows(&state), [RowKey::Session(IssueNumber(3)), RowKey::Session(IssueNumber(2)), RowKey::Session(IssueNumber(1))]);
        press(&mut state, &config, key('/'));
        for c in "docs".chars() {
            press(&mut state, &config, key(c));
        }
        assert_eq!(visible_rows(&state), [RowKey::Session(IssueNumber(2))], "filters by the issue title");
    }

    #[test]
    fn results_of_writes_clear_starting_toast_and_ask_for_an_observation() {
        let (mut state, config, _) = app();
        state.starting.insert(IssueNumber(3));
        let cmds = press(&mut state, &config, Msg::Started { issue: IssueNumber(3), result: Err("boom".to_string()) });
        assert!(state.starting.is_empty());
        assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("boom")));
        assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::ObserveSessions)]));
        let again = press(&mut state, &config, Msg::Stopped { issue: IssueNumber(3), result: Ok(()) });
        assert!(again.is_empty(), "an observation is already in flight");
    }

    #[test]
    fn a_finished_start_shows_its_session_at_once_with_its_warning() {
        let (mut state, config, _) = app();
        state.starting.insert(IssueNumber(3));
        let outcome = StartOutcome { session: session(3, Some((true, false))), warnings: vec!["fetch failed".to_string()] };
        press(&mut state, &config, Msg::Started { issue: IssueNumber(3), result: Ok(outcome) });
        assert!(state.sessions.contains_key(&IssueNumber(3)));
        assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("fetch failed")));
    }

    #[test]
    fn toast_is_stamped_then_expires() {
        let (mut state, config, now) = app();
        say(&mut state, "hi".to_string());
        press(&mut state, &config, Msg::Tick(now));
        assert!(state.toast.is_some());
        press(&mut state, &config, Msg::Tick(now + TOAST_FOR));
        assert!(state.toast.is_none());
    }

    #[test]
    fn r_refreshes_without_stacking_and_q_quits() {
        let (mut state, config, _) = app();
        let cmds = press(&mut state, &config, key('r'));
        assert_eq!(cmds.len(), 3);
        assert!(press(&mut state, &config, key('r')).is_empty());
        assert!(matches!(press(&mut state, &config, key('q')).as_slice(), [Cmd::Quit]));
        assert!(matches!(press(&mut state, &config, ctrl('c')).as_slice(), [Cmd::Quit]));
    }

    fn github_fetches(cmds: &[Cmd]) -> usize {
        cmds.iter().filter(|c| matches!(c, Cmd::Job(Job::FetchIssues | Job::FetchPrs))).count()
    }

    /// Sends `msg`, then checks that the GitHub fetch waits for the cooldown,
    /// goes out once, and pushes the 60 s timer back.
    fn assert_triggers_one_refresh(state: &mut State, config: &Config, msg: Msg) {
        let from = state.due.github_sent;
        assert_eq!(github_fetches(&press(state, config, msg)), 0, "the next tick fetches, not the event");
        assert_eq!(github_fetches(&press(state, config, Msg::Tick(from + GITHUB_COOLDOWN - Duration::from_millis(1)))), 0, "cooldown");
        let at = from + GITHUB_COOLDOWN;
        assert_eq!(github_fetches(&press(state, config, Msg::Tick(at))), 2);
        assert_eq!(state.due.github, at + config.github_refresh, "the timer starts again");
        assert_eq!(github_fetches(&press(state, config, Msg::Tick(at + GITHUB_COOLDOWN))), 0, "inflight, and no new event");
        press(state, config, Msg::Issues(Ok(Vec::new())));
        press(state, config, Msg::Prs(Ok(Vec::new())));
        assert_eq!(github_fetches(&press(state, config, Msg::Tick(at + GITHUB_COOLDOWN * 2))), 0, "one event, one fetch");
    }

    #[test]
    fn a_start_or_a_stop_refreshes_github() {
        let (mut state, config, _) = app();
        let outcome = StartOutcome { session: session(3, Some((true, false))), warnings: Vec::new() };
        assert_triggers_one_refresh(&mut state, &config, Msg::Started { issue: IssueNumber(3), result: Ok(outcome) });
        assert_triggers_one_refresh(&mut state, &config, Msg::Stopped { issue: IssueNumber(3), result: Ok(()) });

        let (mut state, config, now) = app();
        press(&mut state, &config, Msg::Started { issue: IssueNumber(3), result: Err("boom".to_string()) });
        assert_eq!(github_fetches(&press(&mut state, &config, Msg::Tick(now + GITHUB_COOLDOWN))), 0, "a failed start changes nothing");
    }

    #[test]
    fn a_session_that_goes_idle_or_exits_refreshes_github() {
        let observe = |sessions: Vec<Session>| Msg::Sessions(Ok(sessions.into_iter().map(|s| (s.issue, s)).collect()));
        let running = || session(3, Some((true, false)));
        let needs_you = || session(3, Some((true, true)));
        let exited = || session(3, Some((false, false)));
        for (before, after) in [(running(), Some(needs_you())), (running(), Some(exited())), (needs_you(), Some(exited())), (running(), None)] {
            let (mut state, config, _) = app();
            state.sessions.insert(IssueNumber(3), before);
            assert_triggers_one_refresh(&mut state, &config, observe(after.into_iter().collect()));
        }
        for (before, after) in [(running(), running()), (needs_you(), running()), (needs_you(), needs_you()), (exited(), exited())] {
            let (mut state, config, now) = app();
            state.sessions.insert(IssueNumber(3), before);
            press(&mut state, &config, observe(vec![after]));
            assert_eq!(github_fetches(&press(&mut state, &config, Msg::Tick(now + GITHUB_COOLDOWN))), 0);
        }
    }

    #[test]
    fn focus_gained_refreshes_github() {
        let (mut state, config, _) = app();
        assert_triggers_one_refresh(&mut state, &config, Msg::FocusGained);
    }

    #[test]
    fn a_burst_of_events_makes_one_fetch_and_an_event_during_a_fetch_makes_one_more() {
        let (mut state, config, now) = app();
        for _ in 0..3 {
            press(&mut state, &config, Msg::FocusGained);
            press(&mut state, &config, Msg::Stopped { issue: IssueNumber(3), result: Ok(()) });
        }
        let at = now + GITHUB_COOLDOWN;
        assert_eq!(github_fetches(&press(&mut state, &config, Msg::Tick(at))), 2);

        // This fetch may have started before the change, so another one follows it.
        press(&mut state, &config, Msg::FocusGained);
        let later = at + GITHUB_COOLDOWN;
        assert_eq!(github_fetches(&press(&mut state, &config, Msg::Tick(later))), 0, "waits for the fetch in flight");
        press(&mut state, &config, Msg::Issues(Ok(Vec::new())));
        assert_eq!(github_fetches(&press(&mut state, &config, Msg::Tick(later))), 0, "PRs still in flight");
        press(&mut state, &config, Msg::Prs(Ok(Vec::new())));
        assert_eq!(github_fetches(&press(&mut state, &config, Msg::Tick(later))), 2);
    }

    #[test]
    fn the_timer_firing_during_a_fetch_keeps_the_pending_event() {
        let (mut state, config, now) = app();
        state.inflight.issues = true;
        press(&mut state, &config, Msg::FocusGained);
        state.due.github = now + GITHUB_COOLDOWN;
        let cmds = press(&mut state, &config, Msg::Tick(now + GITHUB_COOLDOWN));
        assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::FetchPrs), ..]), "the timer fetches what is idle");
        press(&mut state, &config, Msg::Issues(Ok(Vec::new())));
        press(&mut state, &config, Msg::Prs(Ok(Vec::new())));
        assert_eq!(github_fetches(&press(&mut state, &config, Msg::Tick(now + GITHUB_COOLDOWN * 2))), 2, "the issues fetch predated the event");
    }

    #[test]
    fn key_bar_follows_the_context() {
        let (mut state, config, _) = app();
        let names = |s: &State| key_bar(s).iter().map(|(k, d)| format!("{k} {d}")).collect::<Vec<_>>();
        assert!(names(&state).contains(&"s start session".to_string()));
        assert!(names(&state).contains(&"o open on GitHub".to_string()));
        assert!(!names(&state).iter().any(|k| k.starts_with("x ")));
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        assert!(names(&state).contains(&"a attach".to_string()));
        assert!(names(&state).contains(&"x stop".to_string()));
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        assert!(names(&state).contains(&"a resume".to_string()));
        press(&mut state, &config, key('2'));
        assert!(names(&state).contains(&"enter go to issue".to_string()));
    }

    fn opens(cmds: &[Cmd]) -> Vec<GithubRef> {
        cmds.iter().filter_map(|c| match c { Cmd::Job(Job::Open(t)) => Some(*t), _ => None }).collect()
    }

    #[test]
    fn o_opens_the_selected_issue_pr_or_session_issue() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('j'));
        assert_eq!(opens(&press(&mut state, &config, key('o'))), [GithubRef::Issue(IssueNumber(2))]);
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Opening issue #2 on GitHub"));

        press(&mut state, &config, key('2'));
        press(&mut state, &config, key('j'));
        assert_eq!(opens(&press(&mut state, &config, key('o'))), [GithubRef::Pr(PrNumber(11))]);

        state.sessions.insert(IssueNumber(7), session(7, Some((true, false))));
        press(&mut state, &config, key('3'));
        assert_eq!(opens(&press(&mut state, &config, key('o'))), [GithubRef::Issue(IssueNumber(7))], "an issue not in the list still opens");
    }

    #[test]
    fn o_on_an_empty_list_sends_nothing() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('3'));
        assert!(press(&mut state, &config, key('o')).is_empty());
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Nothing to open."));

        press(&mut state, &config, key('1'));
        press(&mut state, &config, key('/'));
        for c in "zzz".chars() {
            press(&mut state, &config, key(c));
        }
        press(&mut state, &config, code(KeyCode::Enter));
        assert!(press(&mut state, &config, key('o')).is_empty(), "a filter that hides every row");
    }

    #[test]
    fn o_while_filtering_types_into_the_filter() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('/'));
        assert!(press(&mut state, &config, key('o')).is_empty());
        assert_eq!(state.filter, "o");
    }

    #[test]
    fn a_failed_open_shows_the_error_and_a_success_is_quiet() {
        let (mut state, config, _) = app();
        let target = GithubRef::Pr(PrNumber(10));
        assert!(press(&mut state, &config, Msg::Opened { target, result: Ok(()) }).is_empty());
        assert!(state.toast.is_none());
        press(&mut state, &config, Msg::Opened { target, result: Err("no browser".to_string()) });
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Open PR #10 failed: no browser"));
    }

    fn sends(cmds: &[Cmd]) -> Vec<(IssueNumber, &str)> {
        cmds.iter().filter_map(|c| match c { Cmd::Job(Job::Send { issue, text }) => Some((*issue, text.as_str())), _ => None }).collect()
    }

    fn merges(cmds: &[Cmd]) -> Vec<MergeAsk> {
        cmds.iter().filter_map(|c| match c { Cmd::Job(Job::Merge(ask)) => Some(*ask), _ => None }).collect()
    }

    fn toast(state: &State) -> String {
        state.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default()
    }

    const ASK: MergeAsk = MergeAsk { pr: PrNumber(10), new_session_ok: false };
    const ASK_FIX: MergeAsk = MergeAsk { pr: PrNumber(10), new_session_ok: true };

    fn handoff(problem: MergeProblem) -> Msg {
        let mut fresh = pr(10, "fix/3-x", &[3]);
        fresh.mergeability = Mergeability::Conflict;
        Msg::Merged { ask: ASK, result: Ok(MergeOutcome::Handoff { problem, method: MergeMethod::Rebase, pr: fresh }) }
    }

    fn with_ask(msg: Msg, ask: MergeAsk) -> Msg {
        match msg {
            Msg::Merged { result, .. } => Msg::Merged { ask, result },
            other => other,
        }
    }

    #[test]
    fn m_twice_starts_a_merge_job_for_the_session_pr() {
        let (mut state, config, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, true))));
        press(&mut state, &config, key('3'));
        assert!(key_bar(&state).contains(&("m", "merge")));
        assert!(press(&mut state, &config, key('m')).is_empty(), "the first press only arms");
        assert_eq!(toast(&state), "Press m again to merge PR #10");
        let cmds = press(&mut state, &config, key('m'));
        assert_eq!(merges(&cmds), [ASK]);
        assert!(sends(&cmds).is_empty(), "no agent turn for the merge itself");
        assert!(state.armed.is_none());
        assert!(press(&mut state, &config, key('m')).is_empty(), "a third press arms again");
    }

    #[test]
    fn m_merges_without_a_session_from_the_pr_or_the_issue() {
        let (mut state, config, _) = app();
        assert!(key_bar(&state).contains(&("m", "merge")), "issue #3 has PR #10");
        press(&mut state, &config, key('m'));
        assert_eq!(merges(&press(&mut state, &config, key('m'))), [ASK]);

        press(&mut state, &config, key('2'));
        press(&mut state, &config, key('j'));
        press(&mut state, &config, key('m'));
        assert_eq!(toast(&state), "Press m again to merge PR #11");
        assert_eq!(merges(&press(&mut state, &config, key('m'))), [MergeAsk { pr: PrNumber(11), new_session_ok: false }]);
    }

    #[test]
    fn the_first_m_names_the_problem_and_the_agent_of_a_running_session() {
        let (mut state, config, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        state.prs.items[0].mergeability = Mergeability::Conflict;
        press(&mut state, &config, key('3'));
        press(&mut state, &config, key('m'));
        assert_eq!(toast(&state), "PR #10 has a merge conflict with main. Press m again to ask #3's agent to fix and merge it.");
        assert_eq!(merges(&press(&mut state, &config, key('m'))), [ASK], "the job decides, not the cached list");
    }

    #[test]
    fn without_a_running_session_the_first_m_offers_a_new_session() {
        let (mut state, config, _) = app();
        state.prs.items[0].mergeability = Mergeability::Conflict;
        press(&mut state, &config, key('m'));
        assert_eq!(toast(&state), "PR #10 has a merge conflict with main. Press m again to start a session for #3 that fixes it.");
        assert_eq!(merges(&press(&mut state, &config, key('m'))), [ASK_FIX]);
    }

    #[test]
    fn a_problem_found_after_the_user_agreed_starts_a_session_on_the_pr_branch() {
        let (mut state, config, _) = app();
        let cmds = press(&mut state, &config, with_ask(handoff(MergeProblem::Conflict), ASK_FIX));
        let started = starts(&cmds);
        assert_eq!(started.len(), 1);
        let req = started[0];
        assert_eq!((req.issue, req.branch.as_str(), req.mode, req.from), (IssueNumber(3), "fix/3-x", Mode::Implement, StartFrom::Origin));
        assert_eq!(req.prompt, rules::fix_session_prompt(PrNumber(10), "main", MergeProblem::Conflict, MergeMethod::Rebase));
        assert!(state.starting.contains(&IssueNumber(3)));
        assert_eq!((state.tab, state.selected.session), (Tab::Sessions, Some(IssueNumber(3))));
        assert_eq!(toast(&state), "PR #10 has a merge conflict with main. Starting a session for #3 to fix it.");
    }

    #[test]
    fn a_problem_the_list_did_not_show_needs_one_more_m_for_a_new_session() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('m'));
        press(&mut state, &config, key('m'));
        let cmds = press(&mut state, &config, handoff(MergeProblem::Conflict));
        assert!(cmds.is_empty(), "no session before the user agrees");
        assert_eq!(toast(&state), "PR #10 has a merge conflict with main. Press m again to start a session for #3 that fixes it.");
        let started = press(&mut state, &config, key('m'));
        assert_eq!(starts(&started).iter().map(|r| r.issue).collect::<Vec<_>>(), [IssueNumber(3)]);
        assert!(merges(&started).is_empty());
    }

    #[test]
    fn another_key_cancels_the_new_session() {
        let (mut state, config, _) = app();
        press(&mut state, &config, handoff(MergeProblem::Conflict));
        press(&mut state, &config, key('j'));
        press(&mut state, &config, key('k'));
        let cmds = press(&mut state, &config, key('m'));
        assert!(starts(&cmds).is_empty() && merges(&cmds).is_empty());
        assert_eq!(toast(&state), "Press m again to merge PR #10", "the m arms a fresh merge");
    }

    #[test]
    fn an_exited_session_on_the_pr_branch_gets_a_new_session() {
        let (mut state, config, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        let cmds = press(&mut state, &config, with_ask(handoff(MergeProblem::ChecksFailing), ASK_FIX));
        assert!(sends(&cmds).is_empty());
        assert_eq!(starts(&cmds).iter().map(|r| r.issue).collect::<Vec<_>>(), [IssueNumber(3)]);
    }

    #[test]
    fn a_problem_pr_with_no_issue_to_start_on_only_explains() {
        let (mut state, config, _) = app();
        state.prs.items[1].mergeability = Mergeability::Conflict;
        press(&mut state, &config, key('2'));
        press(&mut state, &config, key('j'));
        assert!(press(&mut state, &config, key('m')).is_empty());
        assert_eq!(toast(&state), "PR #11 has a merge conflict with main. PR #11 closes no issue, so dispatch can not start a session for it.");
        assert!(state.armed.is_none());
    }

    #[test]
    fn a_direct_merge_shows_the_method_and_refreshes_the_prs() {
        let (mut state, config, _) = app();
        let cmds = press(&mut state, &config, Msg::Merged { ask: ASK, result: Ok(MergeOutcome::Merged(MergeMethod::Squash)) });
        assert_eq!(toast(&state), "Merged PR #10 (squash)");
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
        assert!(sends(&cmds).is_empty());
    }

    #[test]
    fn a_problem_pr_sends_the_running_agent_a_prompt_with_the_problem_and_method() {
        let (mut state, config, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        let cmds = press(&mut state, &config, handoff(MergeProblem::ChecksFailing));
        let prompt = rules::handoff_prompt(PrNumber(10), "main", MergeProblem::ChecksFailing, MergeMethod::Rebase);
        assert_eq!(sends(&cmds), [(IssueNumber(3), prompt.as_str())]);
        assert!(prompt.contains("failing checks") && prompt.contains("--rebase"));
        assert_eq!(toast(&state), "PR #10 has failing checks. Asking #3's agent to fix it.");
    }

    #[test]
    fn a_queued_merge_says_so() {
        let (mut state, config, _) = app();
        let cmds = press(&mut state, &config, Msg::Merged { ask: ASK, result: Ok(MergeOutcome::Queued) });
        assert_eq!(toast(&state), "PR #10 is in the merge queue");
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
    }

    #[test]
    fn a_refused_or_failed_merge_only_shows_a_toast() {
        let (mut state, config, _) = app();
        let cmds = press(&mut state, &config, Msg::Merged { ask: ASK, result: Ok(MergeOutcome::Refused("wait".to_string())) });
        assert!(cmds.is_empty());
        assert_eq!(toast(&state), "wait");
        let cmds = press(&mut state, &config, Msg::Merged { ask: ASK, result: Err("not allowed".to_string()) });
        assert!(cmds.is_empty());
        assert_eq!(toast(&state), "Merge PR #10 failed: not allowed");
    }

    #[test]
    fn m_from_the_pr_row_reaches_the_same_pr() {
        let (mut state, config, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        press(&mut state, &config, key('2'));
        press(&mut state, &config, key('m'));
        assert_eq!(merges(&press(&mut state, &config, key('m'))), [ASK]);
    }

    #[test]
    fn another_key_between_the_two_m_presses_disarms() {
        let (mut state, config, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        press(&mut state, &config, key('3'));
        press(&mut state, &config, key('m'));
        press(&mut state, &config, key('r'));
        assert!(merges(&press(&mut state, &config, key('m'))).is_empty());
    }

    #[test]
    fn m_explains_why_it_can_not_merge() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('3'));
        press(&mut state, &config, key('m'));
        assert_eq!(toast(&state), "Nothing is selected.");

        press(&mut state, &config, key('1'));
        state.prs.items[0].state = PrState::Draft;
        press(&mut state, &config, key('m'));
        assert_eq!(toast(&state), "PR #10 is a draft.");
        assert!(!key_bar(&state).contains(&("m", "merge")));

        state.prs.items.remove(0);
        for _ in 0..2 {
            assert!(press(&mut state, &config, key('m')).is_empty());
        }
        assert_eq!(toast(&state), "#3 has no PR yet.");
    }

    #[test]
    fn a_sent_result_shows_a_toast() {
        let (mut state, config, _) = app();
        press(&mut state, &config, Msg::Sent { issue: IssueNumber(3), result: Ok(()) });
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Asked #3's agent to fix and merge its PR"));
        press(&mut state, &config, Msg::Sent { issue: IssueNumber(3), result: Err("gone".to_string()) });
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Merge request to #3 failed: gone"));
    }

    #[test]
    fn every_binding_key_is_unique() {
        let mut seen: Vec<Key> = Vec::new();
        for k in BINDINGS.iter().flat_map(|b| b.keys) {
            assert!(!seen.contains(k), "{k:?} is bound twice");
            seen.push(*k);
        }
    }

    #[test]
    fn release_events_are_ignored() {
        let (mut state, config, _) = app();
        let mut ev = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        ev.kind = KeyEventKind::Release;
        assert!(press(&mut state, &config, Msg::Key(ev)).is_empty());
    }
}
