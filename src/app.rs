//! UI state and the reducer. `update` is the only writer of `State`; it does
//! no I/O and returns `Cmd`s for the shell to run. Every key, click, tick, and
//! worker result is a `Msg`, so each behaviour is a unit test:
//! "given this state, this msg -> this state and these cmds".

use crate::config::{Change, Config, SETTINGS, Setting};
use crate::domain::{
    BranchName, GithubRef, Issue, IssueNumber, MergeMethod, Mode, PrNumber, PrState, PullRequest, Split,
};
use crate::merge::{MergeOutcome, MergeRun};
use crate::rules::{self, MergePlan, MergeProblem};
use crate::session::{Launch, Session, SessionState, Sessions, StartFrom, StartOutcome, StartRequest, WindowId};
use crate::sort::{self, Cycle, IssueSort, PrSort, SessionSort};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Issues,
    Prs,
    Sessions,
}

/// PROTOTYPE: which layout draws the screen. `v` cycles it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Classic,
    Agent,
    AgentPreview,
}

impl View {
    fn next(self) -> View {
        match self {
            View::Classic => View::Agent,
            View::Agent => View::AgentPreview,
            View::AgentPreview => View::Classic,
        }
    }
}

/// PROTOTYPE: the section a row sits in on the agent views. Lower `rank` first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group {
    pub rank: u8,
    pub name: &'static str,
}

pub fn group_of(state: &State, key: RowKey) -> Group {
    let g = |rank, name| Group { rank, name };
    let by_session = |n: IssueNumber| match activity(state, n) {
        Some(SessionState::NeedsYou) => Some(g(0, "Needs you")),
        Some(SessionState::Running) => Some(g(1, "Working")),
        Some(SessionState::Exited) | None => None,
    };
    match key {
        RowKey::Session(n) => by_session(n).unwrap_or(g(2, "Exited")),
        RowKey::Issue(n) => by_session(n).unwrap_or_else(|| match lead_pr(state, n).map(|p| p.state) {
            Some(PrState::Open | PrState::Draft) => g(2, "In review"),
            Some(PrState::Merged) => g(3, "Merged"),
            _ => g(4, "Open"),
        }),
        RowKey::Pr(n) => match state.prs.items.iter().find(|p| p.number == n).map(|p| p.state) {
            Some(PrState::Open) => g(0, "Ready for review"),
            Some(PrState::Draft) => g(1, "Draft"),
            Some(PrState::Merged) => g(2, "Merged"),
            _ => g(3, "Closed"),
        },
    }
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
    Settings(Box<SettingsMenu>),
}

/// The settings overlay. Browsing has no `edit`; a typed value has one.
pub struct SettingsMenu {
    pub at: Setting,
    pub edit: Option<SettingEdit>,
}

/// A typed value being edited. The error belongs here, so a browsing menu
/// can not show one.
pub struct SettingEdit {
    pub input: tui_textarea::TextArea<'static>,
    pub error: Option<String>,
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
#[expect(clippy::struct_excessive_bools, reason = "one independent flag per job kind")]
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
    pub open_with: OpenWith,
}

/// What `o` does with a page. Over SSH, `gh --web` opens a browser on the
/// remote machine, so dispatch copies the URL to the local clipboard instead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpenWith {
    #[default]
    Browser,
    Clipboard,
}

pub struct State {
    pub context: Context,
    /// The live config. The settings menu edits it, and `Job::SaveConfig`
    /// writes the change to the file.
    pub config: Config,
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
    pub sort: Sorts,
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
    pub view: View,
    pub summaries: std::collections::BTreeMap<IssueNumber, crate::transcript::Summary>,
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
/// the first `m` told the user so. Otherwise one more `m` must agree. A draft
/// is marked ready only when `ready_ok`, for the same reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeAsk {
    pub pr: PrNumber,
    pub new_session_ok: bool,
    pub ready_ok: bool,
}

/// Who fixes a PR that can not merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fixer {
    /// A running session on the PR's branch. It gets a prompt.
    Agent(IssueNumber),
    /// A new session for this issue, on the PR's branch.
    NewSession(IssueNumber),
}

/// Each tab's sort mode. `S` cycles the shown tab's mode only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sorts {
    pub issues: IssueSort,
    pub prs: PrSort,
    pub sessions: SessionSort,
}

impl Sorts {
    fn cycle(&mut self, tab: Tab) {
        match tab {
            Tab::Issues => self.issues = self.issues.next(),
            Tab::Prs => self.prs = self.prs.next(),
            Tab::Sessions => self.sessions = self.sessions.next(),
        }
    }

    pub fn label(self, tab: Tab) -> &'static str {
        match tab {
            Tab::Issues => self.issues.label(),
            Tab::Prs => self.prs.label(),
            Tab::Sessions => self.sessions.label(),
        }
    }
}

#[derive(Default)]
pub struct Selected {
    pub issue: Option<IssueNumber>,
    pub pr: Option<PrNumber>,
    pub session: Option<IssueNumber>,
}

pub enum Msg {
    /// PROTOTYPE: what each session's Claude last said.
    Summaries(std::collections::BTreeMap<IssueNumber, crate::transcript::Summary>),
    Key(KeyEvent),
    Mouse(MouseEvent, Instant),
    Tick(Instant),
    /// `at` is stamped by the worker when `gh` returns.
    Issues {
        result: Result<Vec<Issue>, String>,
        at: Instant,
    },
    Prs {
        result: Result<Vec<PullRequest>, String>,
        at: Instant,
    },
    Sessions(Result<Sessions, String>),
    Tail {
        window: WindowId,
        result: Result<Vec<String>, String>,
    },
    Started {
        issue: IssueNumber,
        result: Result<StartOutcome, String>,
    },
    Stopped {
        issue: IssueNumber,
        result: Result<(), String>,
    },
    Resumed {
        issue: IssueNumber,
        result: Result<(), String>,
    },
    Attached(Result<(), String>),
    Opened {
        target: GithubRef,
        result: Result<(), String>,
    },
    PageUrl {
        target: GithubRef,
        result: Result<String, String>,
    },
    Copied {
        target: GithubRef,
        result: Result<String, String>,
    },
    Sent {
        issue: IssueNumber,
        result: Result<(), String>,
    },
    Merged {
        ask: MergeAsk,
        run: MergeRun,
    },
    /// A setting reached the config file.
    Saved(Result<(), String>),
    /// The terminal got focus back, often after a merge or comment in the browser.
    FocusGained,
}

/// Work for the shell.
pub enum Cmd {
    Job(Job),
    /// Runs on the main thread: `AttachPlan::Suspend` must own the terminal.
    Attach(WindowId),
    /// Runs on the main thread: the copy writes to the terminal.
    Copy {
        target: GithubRef,
        text: String,
    },
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
    Start(StartRequest, Launch),
    Stop(IssueNumber),
    Resume(IssueNumber, Launch),
    /// Open the page in the browser. No inflight flag: two presses may open
    /// two tabs, which is what the user asked for.
    Open(GithubRef),
    /// Look up the page URL, for `OpenWith::Clipboard`.
    FetchPageUrl(GithubRef),
    /// Save one setting to the config file. It runs on the writer thread, so
    /// two quick saves can not interleave their read and write.
    SaveConfig {
        setting: Setting,
        config: Config,
    },
    /// Type a line into the session's Claude prompt and submit it.
    Send {
        issue: IssueNumber,
        text: String,
    },
    /// Check the PR again, then merge it or report what the agent must fix.
    Merge(MergeAsk),
}

pub const SESSIONS_EVERY: Duration = Duration::from_secs(2);
pub const TAIL_EVERY: Duration = Duration::from_secs(1);
pub const TAIL_LINES: u16 = 60;
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const TOAST_FOR: Duration = Duration::from_secs(5);
const GITHUB_COOLDOWN: Duration = Duration::from_secs(5);

pub fn init(now: Instant, config: Config, context: Context) -> (State, Vec<Cmd>) {
    let state = State {
        context,
        split: config.split,
        tab: Tab::Issues,
        issues: Feed { items: Vec::new(), status: FeedStatus::Loading },
        prs: Feed { items: Vec::new(), status: FeedStatus::Loading },
        sessions: Sessions::new(),
        starting: BTreeSet::new(),
        selected: Selected::default(),
        sort: Sorts::default(),
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
        dragging_split: false,
        armed: None,
        view: if cfg!(test) { View::Classic } else { View::Agent },
        summaries: std::collections::BTreeMap::new(),
        config,
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
    Sort,
    SplitNarrower,
    SplitWider,
    SplitReset,
    Settings,
    View,
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
    /// The selection has an open or draft PR.
    Mergeable,
    OpenWith(OpenWith),
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
    Binding {
        keys: &[Key::Char('j'), Key::Code(KeyCode::Down)],
        action: Action::Down,
        bar: &[bar("j/k", "move", When::Always)],
    },
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
    Binding {
        keys: &[Key::Char('o')],
        action: Action::Open,
        bar: &[
            bar("o", "open on GitHub", When::OpenWith(OpenWith::Browser)),
            bar("o", "copy URL", When::OpenWith(OpenWith::Clipboard)),
        ],
    },
    Binding { keys: &[Key::Char('r')], action: Action::Refresh, bar: &[bar("r", "refresh", When::Always)] },
    Binding { keys: &[Key::Char('/')], action: Action::Filter, bar: &[bar("/", "filter", When::Always)] },
    Binding { keys: &[Key::Code(KeyCode::Esc)], action: Action::ClearFilter, bar: &[] },
    // Not in the key bar: the tab row shows `S sort: <mode>`.
    Binding { keys: &[Key::Char('S')], action: Action::Sort, bar: &[] },
    Binding { keys: &[Key::Char('<')], action: Action::SplitNarrower, bar: &[] },
    Binding { keys: &[Key::Char('>')], action: Action::SplitWider, bar: &[] },
    Binding { keys: &[Key::Char('=')], action: Action::SplitReset, bar: &[] },
    Binding { keys: &[Key::Char('v')], action: Action::View, bar: &[] },
    Binding { keys: &[Key::Char('q'), Key::Ctrl('c')], action: Action::Quit, bar: &[bar("q", "quit", When::Always)] },
    // After quit: a narrow terminal clips the end of the bar, and `q quit` must stay.
    Binding { keys: &[Key::Char(',')], action: Action::Settings, bar: &[bar(",", "settings", When::Always)] },
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
        When::OpenWith(with) => state.context.open_with == with,
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
#[expect(clippy::too_many_lines, reason = "one flat match over every `Msg` reads better split no further")]
pub fn update(state: &mut State, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Key(ev) if ev.kind == KeyEventKind::Release => Vec::new(),
        Msg::Key(ev) => match &state.focus {
            Focus::Dialog(_) => dialog_key(state, &ev),
            Focus::Filter => filter_key(state, &ev),
            Focus::Settings(_) => settings_key(state, &ev),
            Focus::List => {
                let armed = state.armed.take();
                match action_for(&ev) {
                    Some(action) => act(state, action, armed.as_ref()),
                    None => Vec::new(),
                }
            }
        },
        Msg::Mouse(ev, now) => on_mouse(state, ev, now),
        Msg::Tick(now) => tick(state, now),
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
        Msg::Summaries(summaries) => {
            state.summaries = summaries;
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
        Msg::PageUrl { target, result } => match result {
            Ok(text) => vec![Cmd::Copy { target, text }],
            Err(e) => {
                say(state, format!("Copy {target} URL failed: {e}"));
                Vec::new()
            }
        },
        Msg::Copied { target, result } => {
            match result {
                Ok(url) => say(state, format!("Copied {url}")),
                Err(e) => say(state, format!("Copy {target} URL failed: {e}")),
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
        Msg::Merged { ask, run } => merged(state, ask, run),
        Msg::Attached(result) => {
            if let Err(e) = result {
                say(state, format!("attach failed: {e}"));
            }
            observe_now(state)
        }
        Msg::Saved(result) => {
            if let Err(e) = result {
                say(state, format!("Saving settings failed: {e}"));
            }
            Vec::new()
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

fn tick(state: &mut State, now: Instant) -> Vec<Cmd> {
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
        state.due.github = now + state.config.github_refresh;
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
fn act(state: &mut State, action: Action, armed: Option<&Armed>) -> Vec<Cmd> {
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
            let live =
                current_session(state).filter(|s| matches!(s.state(), SessionState::Running | SessionState::NeedsYou));
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
        Action::Sort => {
            // Pin the shown row first, so the first-row fallback can not move it.
            if let Some(key) = selected_row(state) {
                select(state, key);
            }
            state.sort.cycle(state.tab);
        }
        Action::SplitNarrower => state.split = state.split.narrower(state.hits.panes.width),
        Action::SplitWider => state.split = state.split.wider(state.hits.panes.width),
        Action::SplitReset => state.split = state.config.split,
        Action::Settings => state.focus = Focus::Settings(Box::new(SettingsMenu { at: SETTINGS[0], edit: None })),
        Action::View => state.view = state.view.next(),
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
    if let (Some(w), SessionState::Running | SessionState::NeedsYou) = (&session.window, session.state()) {
        vec![Cmd::Attach(w.id.clone())]
    } else {
        say(state, format!("Resuming #{} with claude --continue", issue.0));
        vec![Cmd::Job(Job::Resume(issue, Launch::from_config(&state.config)))]
    }
}

/// `o`: the target is fixed at the key press, so a refresh before the job
/// runs can not change what opens.
fn open_key(state: &mut State) -> Vec<Cmd> {
    let Some(target) = selected_row(state).map(GithubRef::from) else {
        say(state, "Nothing to open.".to_string());
        return Vec::new();
    };
    match state.context.open_with {
        OpenWith::Browser => {
            say(state, format!("Opening {target} on GitHub"));
            vec![Cmd::Job(Job::Open(target))]
        }
        OpenWith::Clipboard => {
            say(state, format!("Copying the {target} URL"));
            vec![Cmd::Job(Job::FetchPageUrl(target))]
        }
    }
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
        RowKey::Session(n) => {
            (state.sessions.get(&n).and_then(|s| pr_for_session(state, s)), format!("#{} has no PR yet.", n.0))
        }
    };
    let pr = pr.ok_or(missing)?;
    match pr.state {
        PrState::Open | PrState::Draft => Ok(pr),
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
fn merge_key(state: &mut State, armed: Option<&Armed>) -> Vec<Cmd> {
    let pr = match merge_target(state) {
        Ok(pr) => pr,
        Err(why) => {
            say(state, why);
            return Vec::new();
        }
    };
    let n = pr.number;
    if let Some(Armed::Fix { pr, req }) = armed
        && *pr == n
    {
        return start_fix(state, req.clone());
    }
    let (new_session_ok, ready_ok, expect) = match (rules::merge_plan(pr), fixer(state, pr)) {
        (MergePlan::Handoff(problem), fix) => {
            let sentence = rules::problem_sentence(n, &pr.base, problem);
            match fix {
                Ok(Fixer::Agent(i)) => {
                    (false, false, format!("{sentence} Press m again to ask #{}'s agent to fix and merge it.", i.0))
                }
                Ok(Fixer::NewSession(i)) => {
                    (true, false, format!("{sentence} Press m again to start a session for #{} that fixes it.", i.0))
                }
                Err(why) => {
                    say(state, format!("{sentence} {why}"));
                    return Vec::new();
                }
            }
        }
        // A cached refusal still arms, because the list can be stale and the
        // job asks GitHub again.
        (MergePlan::Direct | MergePlan::Refuse(_), _) => (false, false, format!("Press m again to merge PR #{}", n.0)),
        (MergePlan::MarkReady, _) => {
            (false, true, format!("PR #{} is a draft. Press m again to mark it ready and merge it.", n.0))
        }
    };
    let ask = MergeAsk { pr: n, new_session_ok, ready_ok };
    if armed != Some(&Armed::Merge(ask)) {
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
    vec![Cmd::Job(Job::Start(req, Launch::from_config(&state.config)))]
}

fn fix_request(
    issue: IssueNumber,
    pr: &PullRequest,
    problem: MergeProblem,
    method: MergeMethod,
) -> Result<StartRequest, String> {
    let branch = BranchName::parse(&pr.head)
        .map_err(|e| format!("PR #{} has the branch name {:?}: {e:?}", pr.number.0, pr.head))?;
    Ok(StartRequest {
        issue,
        branch,
        mode: Mode::Implement,
        prompt: rules::fix_session_prompt(pr.number, &pr.base, problem, method),
        from: StartFrom::Origin,
    })
}

fn merged(state: &mut State, ask: MergeAsk, run: MergeRun) -> Vec<Cmd> {
    let ended_ready = run.marked_ready && !matches!(run.result, Ok(MergeOutcome::Merged(_) | MergeOutcome::Queued));
    let refused = matches!(run.result, Ok(MergeOutcome::Refused(_)));
    let mut cmds = merge_result(state, ask, run.result);
    if ended_ready {
        // dispatch does not undo `gh pr ready`, so the toast must say it ran.
        if let Some(toast) = &mut state.toast {
            let _ = write!(toast.text, " PR #{} is now ready for review.", ask.pr.0);
        }
    }
    if ended_ready || refused {
        refresh_github(state, &mut cmds);
    }
    cmds
}

fn merge_result(state: &mut State, ask: MergeAsk, result: Result<MergeOutcome, String>) -> Vec<Cmd> {
    match result {
        Ok(MergeOutcome::Merged(method)) => {
            let mut cmds = Vec::new();
            match live_session_on_pr(state, ask.pr) {
                Some(issue) => {
                    say(state, format!("Merged PR #{} ({method}) and stopped #{}'s session", ask.pr.0, issue.0));
                    cmds.push(Cmd::Job(Job::Stop(issue)));
                }
                None => say(state, format!("Merged PR #{} ({method})", ask.pr.0)),
            }
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

/// The issue of the session with a live window on the PR's head branch. A
/// merged PR needs no agent, so `merged` stops it like `x` does.
fn live_session_on_pr(state: &State, pr: PrNumber) -> Option<IssueNumber> {
    let head = &state.prs.items.iter().find(|p| p.number == pr)?.head;
    state
        .sessions
        .values()
        .find(|s| s.branch.as_str() == head && matches!(s.state(), SessionState::Running | SessionState::NeedsYou))
        .map(|s| s.issue)
}

/// The check found `problem` on `pr`, the PR as GitHub showed it now.
fn handoff(state: &mut State, ask: MergeAsk, pr: &PullRequest, problem: MergeProblem, method: MergeMethod) -> Vec<Cmd> {
    let sentence = rules::problem_sentence(pr.number, &pr.base, problem);
    let issue = match fixer(state, pr) {
        Ok(Fixer::Agent(i)) => {
            say(state, format!("{sentence} Asking #{}'s agent to fix it.", i.0));
            return vec![Cmd::Job(Job::Send {
                issue: i,
                text: rules::handoff_prompt(pr.number, &pr.base, problem, method),
            })];
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

fn on_mouse(state: &mut State, ev: MouseEvent, now: Instant) -> Vec<Cmd> {
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

// ---- the settings menu -------------------------------------------------------

fn settings_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Settings(menu) = &mut state.focus else { return Vec::new() };
    let key = key_of(ev);
    let Some(edit) = &mut menu.edit else {
        let at = SETTINGS.iter().position(|s| *s == menu.at).unwrap_or(0);
        match key {
            Some(Key::Char('j') | Key::Code(KeyCode::Down)) => menu.at = SETTINGS[(at + 1).min(SETTINGS.len() - 1)],
            Some(Key::Char('k') | Key::Code(KeyCode::Up)) => menu.at = SETTINGS[at.saturating_sub(1)],
            Some(Key::Code(KeyCode::Esc) | Key::Char(',')) => state.focus = Focus::List,
            Some(Key::Code(KeyCode::Enter)) => {
                let setting = menu.at;
                match setting.change(&mut state.config) {
                    Change::Applied => return changed(state, setting),
                    Change::Edit(text) => {
                        let mut input = tui_textarea::TextArea::new(vec![text]);
                        input.move_cursor(tui_textarea::CursorMove::End);
                        menu.edit = Some(SettingEdit { input, error: None });
                    }
                }
            }
            _ => {}
        }
        return Vec::new();
    };
    match key {
        Some(Key::Code(KeyCode::Esc)) => menu.edit = None,
        Some(Key::Code(KeyCode::Enter)) => {
            let setting = menu.at;
            match setting.set_text(&mut state.config, &edit.input.lines().join("")) {
                Ok(()) => {
                    menu.edit = None;
                    return changed(state, setting);
                }
                Err(e) => edit.error = Some(e.to_string()),
            }
        }
        _ => {
            edit.input.input(*ev);
            edit.error = None;
        }
    }
    Vec::new()
}

/// `state.config` already holds the new value. Save it, and move what is live.
fn changed(state: &mut State, setting: Setting) -> Vec<Cmd> {
    if setting == Setting::Split {
        state.split = state.config.split;
    }
    vec![Cmd::Job(Job::SaveConfig { setting, config: state.config.clone() })]
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
        Some(Key::Code(KeyCode::Enter)) if ev.modifiers.contains(KeyModifiers::CONTROL) => {
            return confirm_dialog(state);
        }
        Some(Key::Code(KeyCode::Tab)) => dialog.field = step_field(dialog.field, 1),
        Some(Key::Code(KeyCode::BackTab)) => dialog.field = step_field(dialog.field, FIELDS.len() - 1),
        Some(Key::Code(KeyCode::Down)) if dialog.field != DialogField::Prompt => {
            dialog.field = step_field(dialog.field, 1);
        }
        Some(Key::Code(KeyCode::Up)) if dialog.field != DialogField::Prompt => {
            dialog.field = step_field(dialog.field, FIELDS.len() - 1);
        }
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
    let req = StartRequest { issue: dialog.issue, branch, mode: dialog.mode, prompt, from: StartFrom::Base };
    let n = req.issue;
    state.starting.insert(n);
    state.focus = Focus::List;
    goto_session(state, n);
    say(state, format!("Starting #{}", n.0));
    vec![Cmd::Job(Job::Start(req, Launch::from_config(&state.config)))]
}

// ---- pure view helpers, shared by update and ui ----------------------------

/// Rows of the current tab after the filter, in the tab's sort order. The
/// single definition of "what is in the list"; ui renders it and update moves
/// through it.
pub fn visible_rows(state: &State) -> Vec<RowKey> {
    let mut rows = sorted_rows(state);
    if state.view != View::Classic {
        rows.sort_by_key(|k| group_of(state, *k).rank);
    }
    rows
}

fn sorted_rows(state: &State) -> Vec<RowKey> {
    let f = state.filter.as_str();
    let activity = |n| activity(state, n);
    match state.tab {
        Tab::Issues => {
            let mut rows: Vec<&Issue> = state
                .issues
                .items
                .iter()
                .filter(|i| {
                    let number = format!("#{}", i.number.0);
                    let mut parts = vec![number.as_str(), i.title.as_str()];
                    parts.extend(i.labels.iter().map(String::as_str));
                    rules::matches_filter(&parts, f)
                })
                .collect();
            sort::issues(&mut rows, state.sort.issues, activity);
            rows.into_iter().map(|i| RowKey::Issue(i.number)).collect()
        }
        Tab::Prs => {
            let mut rows: Vec<&PullRequest> = state
                .prs
                .items
                .iter()
                .filter(|p| rules::matches_filter(&[&format!("#{}", p.number.0), &p.title, &p.head], f))
                .collect();
            sort::prs(&mut rows, state.sort.prs);
            rows.into_iter().map(|p| RowKey::Pr(p.number)).collect()
        }
        Tab::Sessions => {
            let numbers: BTreeSet<IssueNumber> = state.sessions.keys().chain(state.starting.iter()).copied().collect();
            let mut rows: Vec<IssueNumber> = numbers
                .into_iter()
                .filter(|n| {
                    let title = state.issues.items.iter().find(|i| i.number == *n).map_or("", |i| i.title.as_str());
                    let branch = state.sessions.get(n).map_or("", |s| s.branch.as_str());
                    rules::matches_filter(&[&format!("#{}", n.0), title, branch], f)
                })
                .collect();
            sort::sessions(&mut rows, state.sort.sessions, activity);
            rows.into_iter().map(RowKey::Session).collect()
        }
    }
}

/// The state of issue `n`'s session for sorting. A start in flight counts as
/// running, so the row does not jump when the window appears.
fn activity(state: &State, n: IssueNumber) -> Option<SessionState> {
    match state.sessions.get(&n) {
        Some(s) => Some(s.state()),
        None => state.starting.contains(&n).then_some(SessionState::Running),
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
    use crate::glyphs::IconSet;
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
        Msg::Mouse(
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: col,
                row,
                modifiers: KeyModifiers::NONE,
            },
            at,
        )
    }

    fn session(n: u32, window: Option<(bool, bool)>) -> Session {
        Session {
            issue: IssueNumber(n),
            branch: BranchName::parse(&format!("fix/{n}-x")).expect("branch"),
            worktree: WorktreePath::planned(PathBuf::from(format!("/w/orbit-{n}"))),
            window: window.map(|(alive, waiting)| Window {
                id: WindowId::parse(&format!("@{n}")).expect("id"),
                alive,
                waiting,
            }),
        }
    }

    /// Three issues (#3 bug, #2 docs, #1 feature) and two open PRs (#10
    /// newer, #11), idle jobs.
    fn app() -> (State, Instant) {
        let now = Instant::now();
        let (mut state, _) = init(now, Config::default(), Context::default());
        state.issues = Feed {
            items: vec![
                issue(3, "Retry queue drops jobs", &["bug"]),
                issue(2, "Docs for plugins", &["docs"]),
                issue(1, "Add json output", &["feature"]),
            ],
            status: FeedStatus::Fresh { at: now },
        };
        let newer = jiff::Timestamp::from_second(60).expect("in range");
        let items = vec![PullRequest { created_at: newer, ..pr(10, "fix/3-x", &[3]) }, pr(11, "other", &[])];
        state.prs = Feed { items, status: FeedStatus::Fresh { at: now } };
        state.inflight = Inflight::default();
        (state, now)
    }

    fn press(state: &mut State, msg: Msg) -> Vec<Cmd> {
        update(state, msg)
    }

    fn dialog(state: &State) -> &StartDialog {
        match &state.focus {
            Focus::Dialog(d) => d,
            _ => panic!("no dialog is open"),
        }
    }

    fn starts(cmds: &[Cmd]) -> Vec<&StartRequest> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::Job(Job::Start(r, _)) => Some(r),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn init_asks_for_all_three_reads() {
        let (state, cmds) = init(Instant::now(), Config::default(), Context::default());
        let jobs: Vec<_> = cmds.iter().map(|c| matches!(c, Cmd::Job(_))).collect();
        assert_eq!(jobs, [true, true, true]);
        assert!(state.inflight.issues && state.inflight.prs && state.inflight.sessions);
    }

    #[test]
    fn s_on_an_issue_opens_the_dialog_with_label_defaults() {
        let (mut state, _) = app();
        assert!(press(&mut state, key('s')).is_empty());
        let d = dialog(&state);
        assert_eq!(d.branch.lines(), ["fix/3-retry-queue-drops-jobs"]);
        assert!(d.prompt.lines()[0].starts_with("Implement GitHub issue #3: Retry queue drops jobs"));
    }

    #[test]
    fn s_on_an_issue_with_a_session_goes_to_it_and_opens_no_dialog() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        let cmds = press(&mut state, key('s'));
        assert!(cmds.is_empty());
        assert_eq!(state.tab, Tab::Sessions);
        assert_eq!(selected_row(&state), Some(RowKey::Session(IssueNumber(3))));
        assert!(matches!(state.focus, Focus::List));
    }

    #[test]
    fn confirm_with_an_invalid_branch_keeps_the_dialog_open_with_an_error() {
        let (mut state, _) = app();
        press(&mut state, key('s'));
        press(&mut state, key('x'));
        let Focus::Dialog(d) = &mut state.focus else { panic!("dialog") };
        d.branch.insert_str(" bad name");
        let cmds = press(&mut state, ctrl('s'));
        assert!(cmds.is_empty());
        assert!(dialog(&state).error.is_some());
        assert!(state.starting.is_empty());
    }

    #[test]
    fn confirm_starts_once_and_a_second_s_emits_no_start() {
        let (mut state, _) = app();
        press(&mut state, key('s'));
        let cmds = press(&mut state, ctrl('s'));
        let reqs = starts(&cmds);
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].issue, IssueNumber(3));
        assert_eq!(reqs[0].branch.as_str(), "fix/3-retry-queue-drops-jobs");
        assert!(reqs[0].prompt.starts_with("Implement GitHub issue #3"));
        assert_eq!(state.tab, Tab::Sessions);
        assert_eq!(visible_rows(&state), [RowKey::Session(IssueNumber(3))]);

        press(&mut state, key('1'));
        let again = press(&mut state, key('s'));
        assert!(again.is_empty());
        assert!(matches!(state.focus, Focus::List), "no dialog while starting");
    }

    #[test]
    fn enter_confirms_on_the_branch_field_but_adds_a_newline_in_the_prompt() {
        let (mut state, _) = app();
        press(&mut state, key('s'));
        for _ in 0..2 {
            press(&mut state, code(KeyCode::Tab));
        }
        assert_eq!(dialog(&state).field, DialogField::Prompt);
        let lines = dialog(&state).prompt.lines().len();
        assert!(press(&mut state, code(KeyCode::Enter)).is_empty());
        assert_eq!(dialog(&state).prompt.lines().len(), lines + 1);
        press(&mut state, code(KeyCode::Tab));
        assert_eq!(dialog(&state).field, DialogField::Branch);
        let cmds = press(&mut state, code(KeyCode::Enter));
        assert_eq!(starts(&cmds).len(), 1);
    }

    #[test]
    fn dialog_mode_toggles_and_reaches_the_request() {
        let (mut state, _) = app();
        press(&mut state, key('s'));
        press(&mut state, code(KeyCode::BackTab));
        assert_eq!(dialog(&state).field, DialogField::Prompt);
        press(&mut state, code(KeyCode::Tab));
        press(&mut state, code(KeyCode::Tab));
        assert_eq!(dialog(&state).field, DialogField::Mode);
        press(&mut state, code(KeyCode::Right));
        assert_eq!(dialog(&state).mode, Mode::PlanFirst);
        let cmds = press(&mut state, ctrl('s'));
        assert_eq!(starts(&cmds)[0].mode, Mode::PlanFirst);
    }

    #[test]
    fn esc_closes_the_dialog_without_a_command() {
        let (mut state, _) = app();
        press(&mut state, key('s'));
        assert!(press(&mut state, code(KeyCode::Esc)).is_empty());
        assert!(matches!(state.focus, Focus::List));
        assert!(state.starting.is_empty());
    }

    #[test]
    fn tick_does_not_stack_a_second_fetch_while_one_is_inflight() {
        let (mut state, now) = app();
        state.due.github = now;
        state.inflight.issues = true;
        let cmds = press(&mut state, Msg::Tick(now + Duration::from_millis(1)));
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchIssues))));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
        let later = press(&mut state, Msg::Tick(now + Duration::from_millis(2)));
        assert!(later.is_empty(), "due time moved forward");
    }

    #[test]
    fn tick_observes_sessions_every_two_seconds_once() {
        let (mut state, now) = app();
        let t = now + SESSIONS_EVERY;
        let cmds = press(&mut state, Msg::Tick(t));
        assert_eq!(cmds.iter().filter(|c| matches!(c, Cmd::Job(Job::ObserveSessions))).count(), 1);
        let cmds = press(&mut state, Msg::Tick(t + Duration::from_millis(100)));
        assert!(cmds.is_empty());
    }

    #[test]
    fn issues_error_keeps_the_old_items_and_stamps_the_failure() {
        let (mut state, now) = app();
        state.inflight.issues = true;
        let failed = now + Duration::from_secs(5);
        press(&mut state, Msg::Issues { result: Err("offline".to_string()), at: failed });
        assert_eq!(state.issues.items.len(), 3);
        assert!(
            matches!(&state.issues.status, FeedStatus::Failed { error, at } if error == "offline" && *at == failed)
        );
        assert!(!state.inflight.issues);
        let arrived = now + Duration::from_secs(9);
        press(&mut state, Msg::Issues { result: Ok(vec![issue(9, "new", &[])]), at: arrived });
        assert_eq!(state.issues.items.len(), 1);
        assert!(matches!(state.issues.status, FeedStatus::Fresh { at } if at == arrived));
    }

    #[test]
    fn new_data_resets_the_refresh_time_on_the_timer_and_after_r() {
        let (mut state, _) = app();
        let due = state.due.github;
        let cmds = press(&mut state, Msg::Tick(due));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
        let first = due + Duration::from_secs(2);
        press(&mut state, Msg::Prs { result: Ok(Vec::new()), at: first });
        assert!(matches!(state.prs.status, FeedStatus::Fresh { at } if at == first));

        let cmds = press(&mut state, key('r'));
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
        let second = first + Duration::from_secs(30);
        press(&mut state, Msg::Prs { result: Ok(Vec::new()), at: second });
        assert!(matches!(state.prs.status, FeedStatus::Fresh { at } if at == second));
    }

    fn hit_state() -> (State, Instant) {
        let (mut state, now) = app();
        state.hits.targets = vec![
            (Rect::new(0, 0, 10, 1), HitTarget::Tab(Tab::Issues)),
            (Rect::new(10, 0, 10, 1), HitTarget::Tab(Tab::Prs)),
            (Rect::new(0, 2, 30, 1), HitTarget::Row(RowKey::Issue(IssueNumber(3)))),
            (Rect::new(0, 3, 30, 1), HitTarget::Row(RowKey::Issue(IssueNumber(2)))),
        ];
        (state, now)
    }

    #[test]
    fn click_selects_a_row_and_a_tab() {
        let (mut state, now) = hit_state();
        press(&mut state, click(5, 3, now));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
        press(&mut state, click(12, 0, now));
        assert_eq!(state.tab, Tab::Prs);
    }

    #[test]
    fn double_click_inside_the_window_opens_the_dialog_and_outside_only_selects() {
        let (mut state, now) = hit_state();
        press(&mut state, click(5, 3, now));
        press(&mut state, click(5, 3, now + DOUBLE_CLICK.saturating_sub(Duration::from_millis(1))));
        assert_eq!(dialog(&state).issue, IssueNumber(2));

        let (mut state, now) = hit_state();
        press(&mut state, click(5, 3, now));
        press(&mut state, click(5, 3, now + DOUBLE_CLICK + Duration::from_millis(1)));
        assert!(matches!(state.focus, Focus::List));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
    }

    #[test]
    fn wheel_moves_the_selection() {
        let (mut state, now) = app();
        let wheel = |kind| Msg::Mouse(MouseEvent { kind, column: 0, row: 0, modifiers: KeyModifiers::NONE }, now);
        press(&mut state, wheel(MouseEventKind::ScrollDown));
        press(&mut state, wheel(MouseEventKind::ScrollDown));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
        press(&mut state, wheel(MouseEventKind::ScrollDown));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))), "clamped at the end");
        press(&mut state, wheel(MouseEventKind::ScrollUp));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
    }

    #[test]
    fn jk_move_and_stay_inside_the_list() {
        let (mut state, _) = app();
        press(&mut state, key('k'));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(3))));
        press(&mut state, key('j'));
        press(&mut state, code(KeyCode::Down));
        press(&mut state, key('j'));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
    }

    #[test]
    fn a_filter_that_hides_the_selection_selects_the_first_visible_and_clearing_restores_it() {
        let (mut state, _) = app();
        press(&mut state, key('j'));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
        press(&mut state, key('/'));
        assert!(matches!(state.focus, Focus::Filter));
        for c in "json".chars() {
            press(&mut state, key(c));
        }
        press(&mut state, code(KeyCode::Enter));
        assert_eq!(visible_rows(&state), [RowKey::Issue(IssueNumber(1))]);
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
        press(&mut state, code(KeyCode::Esc));
        assert_eq!(visible_rows(&state).len(), 3);
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
    }

    #[test]
    fn filter_esc_clears_and_backspace_edits() {
        let (mut state, _) = app();
        press(&mut state, key('/'));
        press(&mut state, key('a'));
        press(&mut state, key('b'));
        press(&mut state, code(KeyCode::Backspace));
        assert_eq!(state.filter, "a");
        press(&mut state, code(KeyCode::Esc));
        assert_eq!((state.filter.as_str(), matches!(state.focus, Focus::List)), ("", true));
    }

    #[test]
    fn stale_tail_is_dropped_and_current_tail_is_kept() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        state.inflight.tail = true;
        let old_window = WindowId::parse("@99").expect("id");
        press(&mut state, Msg::Tail { window: old_window, result: Ok(vec!["old".to_string()]) });
        assert!(state.tail.is_none());
        assert!(!state.inflight.tail);
        let new_window = WindowId::parse("@3").expect("id");
        press(&mut state, Msg::Tail { window: new_window, result: Ok(vec!["new".to_string()]) });
        assert_eq!(state.tail.as_ref().map(|t| t.lines.clone()), Some(vec!["new".to_string()]));
    }

    #[test]
    fn tick_asks_for_the_tail_of_the_session_in_view_only() {
        let (mut state, now) = app();
        let cmds = press(&mut state, Msg::Tick(now));
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Tail(_)))));
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        let cmds = press(&mut state, Msg::Tick(now));
        assert!(
            cmds.iter().any(|c| matches!(c, Cmd::Job(Job::Tail(id)) if id.as_str() == "@3")),
            "dead panes have a tail too"
        );
    }

    #[test]
    fn a_attaches_a_live_session_and_resumes_an_exited_one() {
        let (mut state, _) = app();
        let cmds = press(&mut state, key('a'));
        assert!(cmds.is_empty());
        assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("Press s")));

        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        let cmds = press(&mut state, key('a'));
        assert!(matches!(cmds.as_slice(), [Cmd::Attach(id)] if id.as_str() == "@3"));

        state.sessions.insert(IssueNumber(3), session(3, Some((true, true))));
        assert!(matches!(press(&mut state, key('a')).as_slice(), [Cmd::Attach(_)]), "needs-you attaches too");

        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        let cmds = press(&mut state, key('a'));
        assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Resume(n, _))] if *n == IssueNumber(3)));
        assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("claude --continue")));
    }

    #[test]
    fn x_stops_only_a_live_session_by_issue_number() {
        let (mut state, _) = app();
        assert!(press(&mut state, key('x')).is_empty());
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        assert!(press(&mut state, key('x')).is_empty(), "exited has nothing to stop");
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        let cmds = press(&mut state, key('x'));
        assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::Stop(n))] if *n == IssueNumber(3)));
    }

    #[test]
    fn pr_enter_goes_to_the_closed_issue_and_pr_rows_show_the_session() {
        let (mut state, _) = app();
        press(&mut state, key('2'));
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        assert_eq!(current_session(&state).map(|s| s.issue), Some(IssueNumber(3)), "via closes");
        press(&mut state, code(KeyCode::Enter));
        assert_eq!(state.tab, Tab::Issues);
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(3))));

        press(&mut state, key('2'));
        press(&mut state, key('j'));
        assert_eq!(selected_row(&state), Some(RowKey::Pr(PrNumber(11))));
        press(&mut state, code(KeyCode::Enter));
        assert_eq!(state.tab, Tab::Prs, "a PR without closes stays");
    }

    #[test]
    fn lead_pr_prefers_merged_then_open_then_draft_and_skips_closed() {
        let (mut state, _) = app();
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
        let (state, _) = app();
        let s = session(3, None);
        assert_eq!(pr_for_session(&state, &s).map(|p| p.number), Some(PrNumber(10)));
        assert!(pr_for_session(&state, &session(4, None)).is_none());
    }

    #[test]
    fn sessions_tab_lists_running_and_starting_before_exited_by_number() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(1), session(1, None));
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        state.starting.insert(IssueNumber(2));
        press(&mut state, key('3'));
        assert_eq!(
            visible_rows(&state),
            [RowKey::Session(IssueNumber(2)), RowKey::Session(IssueNumber(3)), RowKey::Session(IssueNumber(1))]
        );
        press(&mut state, key('/'));
        for c in "docs".chars() {
            press(&mut state, key(c));
        }
        assert_eq!(visible_rows(&state), [RowKey::Session(IssueNumber(2))], "filters by the issue title");
    }

    #[test]
    fn results_of_writes_clear_starting_toast_and_ask_for_an_observation() {
        let (mut state, _) = app();
        state.starting.insert(IssueNumber(3));
        let cmds = press(&mut state, Msg::Started { issue: IssueNumber(3), result: Err("boom".to_string()) });
        assert!(state.starting.is_empty());
        assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("boom")));
        assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::ObserveSessions)]));
        let again = press(&mut state, Msg::Stopped { issue: IssueNumber(3), result: Ok(()) });
        assert!(again.is_empty(), "an observation is already in flight");
    }

    #[test]
    fn a_finished_start_shows_its_session_at_once_with_its_warning() {
        let (mut state, _) = app();
        state.starting.insert(IssueNumber(3));
        let outcome =
            StartOutcome { session: session(3, Some((true, false))), warnings: vec!["fetch failed".to_string()] };
        press(&mut state, Msg::Started { issue: IssueNumber(3), result: Ok(outcome) });
        assert!(state.sessions.contains_key(&IssueNumber(3)));
        assert!(state.toast.as_ref().is_some_and(|t| t.text.contains("fetch failed")));
    }

    #[test]
    fn toast_is_stamped_then_expires() {
        let (mut state, now) = app();
        say(&mut state, "hi".to_string());
        press(&mut state, Msg::Tick(now));
        assert!(state.toast.is_some());
        press(&mut state, Msg::Tick(now + TOAST_FOR));
        assert!(state.toast.is_none());
    }

    #[test]
    fn r_refreshes_without_stacking_and_q_quits() {
        let (mut state, _) = app();
        let cmds = press(&mut state, key('r'));
        assert_eq!(cmds.len(), 3);
        assert!(press(&mut state, key('r')).is_empty());
        assert!(matches!(press(&mut state, key('q')).as_slice(), [Cmd::Quit]));
        assert!(matches!(press(&mut state, ctrl('c')).as_slice(), [Cmd::Quit]));
    }

    fn github_fetches(cmds: &[Cmd]) -> usize {
        cmds.iter().filter(|c| matches!(c, Cmd::Job(Job::FetchIssues | Job::FetchPrs))).count()
    }

    /// Sends `msg`, then checks that the GitHub fetch waits for the cooldown,
    /// goes out once, and pushes the 60 s timer back.
    fn assert_triggers_one_refresh(state: &mut State, msg: Msg) {
        let from = state.due.github_sent;
        assert_eq!(github_fetches(&press(state, msg)), 0, "the next tick fetches, not the event");
        assert_eq!(
            github_fetches(&press(state, Msg::Tick(from + GITHUB_COOLDOWN.saturating_sub(Duration::from_millis(1))))),
            0,
            "cooldown"
        );
        let at = from + GITHUB_COOLDOWN;
        assert_eq!(github_fetches(&press(state, Msg::Tick(at))), 2);
        assert_eq!(state.due.github, at + state.config.github_refresh, "the timer starts again");
        assert_eq!(github_fetches(&press(state, Msg::Tick(at + GITHUB_COOLDOWN))), 0, "inflight, and no new event");
        press(state, Msg::Issues { result: Ok(Vec::new()), at });
        press(state, Msg::Prs { result: Ok(Vec::new()), at });
        assert_eq!(github_fetches(&press(state, Msg::Tick(at + GITHUB_COOLDOWN * 2))), 0, "one event, one fetch");
    }

    #[test]
    fn a_start_or_a_stop_refreshes_github() {
        let (mut state, _) = app();
        let outcome = StartOutcome { session: session(3, Some((true, false))), warnings: Vec::new() };
        assert_triggers_one_refresh(&mut state, Msg::Started { issue: IssueNumber(3), result: Ok(outcome) });
        assert_triggers_one_refresh(&mut state, Msg::Stopped { issue: IssueNumber(3), result: Ok(()) });

        let (mut state, now) = app();
        press(&mut state, Msg::Started { issue: IssueNumber(3), result: Err("boom".to_string()) });
        assert_eq!(
            github_fetches(&press(&mut state, Msg::Tick(now + GITHUB_COOLDOWN))),
            0,
            "a failed start changes nothing"
        );
    }

    #[test]
    fn a_session_that_goes_idle_or_exits_refreshes_github() {
        let observe = |sessions: Vec<Session>| Msg::Sessions(Ok(sessions.into_iter().map(|s| (s.issue, s)).collect()));
        let running = || session(3, Some((true, false)));
        let needs_you = || session(3, Some((true, true)));
        let exited = || session(3, Some((false, false)));
        for (before, after) in [
            (running(), Some(needs_you())),
            (running(), Some(exited())),
            (needs_you(), Some(exited())),
            (running(), None),
        ] {
            let (mut state, _) = app();
            state.sessions.insert(IssueNumber(3), before);
            assert_triggers_one_refresh(&mut state, observe(after.into_iter().collect()));
        }
        for (before, after) in
            [(running(), running()), (needs_you(), running()), (needs_you(), needs_you()), (exited(), exited())]
        {
            let (mut state, now) = app();
            state.sessions.insert(IssueNumber(3), before);
            press(&mut state, observe(vec![after]));
            assert_eq!(github_fetches(&press(&mut state, Msg::Tick(now + GITHUB_COOLDOWN))), 0);
        }
    }

    #[test]
    fn focus_gained_refreshes_github() {
        let (mut state, _) = app();
        assert_triggers_one_refresh(&mut state, Msg::FocusGained);
    }

    #[test]
    fn a_burst_of_events_makes_one_fetch_and_an_event_during_a_fetch_makes_one_more() {
        let (mut state, now) = app();
        for _ in 0..3 {
            press(&mut state, Msg::FocusGained);
            press(&mut state, Msg::Stopped { issue: IssueNumber(3), result: Ok(()) });
        }
        let at = now + GITHUB_COOLDOWN;
        assert_eq!(github_fetches(&press(&mut state, Msg::Tick(at))), 2);

        // This fetch may have started before the change, so another one follows it.
        press(&mut state, Msg::FocusGained);
        let later = at + GITHUB_COOLDOWN;
        assert_eq!(github_fetches(&press(&mut state, Msg::Tick(later))), 0, "waits for the fetch in flight");
        press(&mut state, Msg::Issues { result: Ok(Vec::new()), at: later });
        assert_eq!(github_fetches(&press(&mut state, Msg::Tick(later))), 0, "PRs still in flight");
        press(&mut state, Msg::Prs { result: Ok(Vec::new()), at: later });
        assert_eq!(github_fetches(&press(&mut state, Msg::Tick(later))), 2);
    }

    #[test]
    fn the_timer_firing_during_a_fetch_keeps_the_pending_event() {
        let (mut state, now) = app();
        state.inflight.issues = true;
        press(&mut state, Msg::FocusGained);
        state.due.github = now + GITHUB_COOLDOWN;
        let cmds = press(&mut state, Msg::Tick(now + GITHUB_COOLDOWN));
        assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::FetchPrs), ..]), "the timer fetches what is idle");
        press(&mut state, Msg::Issues { result: Ok(Vec::new()), at: now + GITHUB_COOLDOWN });
        press(&mut state, Msg::Prs { result: Ok(Vec::new()), at: now + GITHUB_COOLDOWN });
        assert_eq!(
            github_fetches(&press(&mut state, Msg::Tick(now + GITHUB_COOLDOWN * 2))),
            2,
            "the issues fetch predated the event"
        );
    }

    #[test]
    fn key_bar_follows_the_context() {
        let (mut state, _) = app();
        let names = |s: &State| key_bar(s).iter().map(|(k, d)| format!("{k} {d}")).collect::<Vec<_>>();
        assert!(names(&state).contains(&"s start session".to_string()));
        assert!(names(&state).contains(&"o open on GitHub".to_string()));
        assert!(!names(&state).iter().any(|k| k.starts_with("x ")));
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        assert!(names(&state).contains(&"a attach".to_string()));
        assert!(names(&state).contains(&"x stop".to_string()));
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        assert!(names(&state).contains(&"a resume".to_string()));
        press(&mut state, key('2'));
        assert!(names(&state).contains(&"enter go to issue".to_string()));
    }

    fn opens(cmds: &[Cmd]) -> Vec<GithubRef> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::Job(Job::Open(t)) => Some(*t),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn o_opens_the_selected_issue_pr_or_session_issue() {
        let (mut state, _) = app();
        press(&mut state, key('j'));
        assert_eq!(opens(&press(&mut state, key('o'))), [GithubRef::Issue(IssueNumber(2))]);
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Opening issue #2 on GitHub"));

        press(&mut state, key('2'));
        press(&mut state, key('j'));
        assert_eq!(opens(&press(&mut state, key('o'))), [GithubRef::Pr(PrNumber(11))]);

        state.sessions.insert(IssueNumber(7), session(7, Some((true, false))));
        press(&mut state, key('3'));
        assert_eq!(
            opens(&press(&mut state, key('o'))),
            [GithubRef::Issue(IssueNumber(7))],
            "an issue not in the list still opens"
        );
    }

    #[test]
    fn o_on_an_empty_list_sends_nothing() {
        let (mut state, _) = app();
        press(&mut state, key('3'));
        assert!(press(&mut state, key('o')).is_empty());
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Nothing to open."));

        press(&mut state, key('1'));
        press(&mut state, key('/'));
        for c in "zzz".chars() {
            press(&mut state, key(c));
        }
        press(&mut state, code(KeyCode::Enter));
        assert!(press(&mut state, key('o')).is_empty(), "a filter that hides every row");
    }

    #[test]
    fn o_while_filtering_types_into_the_filter() {
        let (mut state, _) = app();
        press(&mut state, key('/'));
        assert!(press(&mut state, key('o')).is_empty());
        assert_eq!(state.filter, "o");
    }

    #[test]
    fn o_over_ssh_copies_the_url_instead_of_opening_a_browser() {
        let (mut state, _) = app();
        state.context.open_with = OpenWith::Clipboard;
        press(&mut state, key('j'));
        let cmds = press(&mut state, key('o'));
        assert!(matches!(cmds.as_slice(), [Cmd::Job(Job::FetchPageUrl(GithubRef::Issue(IssueNumber(2))))]));
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Copying the issue #2 URL"));
        let bar: Vec<_> = key_bar(&state).into_iter().filter(|(k, _)| *k == "o").collect();
        assert_eq!(bar, [("o", "copy URL")]);

        let target = GithubRef::Issue(IssueNumber(2));
        let url = "https://github.com/o/r/issues/2".to_string();
        let cmds = press(&mut state, Msg::PageUrl { target, result: Ok(url.clone()) });
        assert!(matches!(cmds.as_slice(), [Cmd::Copy { target: t, text }] if *t == target && *text == url));

        press(&mut state, Msg::Copied { target, result: Ok(url) });
        assert!(
            state.toast.as_ref().is_some_and(|t| t.text == "Copied https://github.com/o/r/issues/2"),
            "the toast shows the URL, because a terminal can ignore OSC 52 without an error"
        );
    }

    #[test]
    fn a_failed_url_lookup_or_copy_shows_the_error() {
        let (mut state, _) = app();
        let target = GithubRef::Pr(PrNumber(10));
        assert!(press(&mut state, Msg::PageUrl { target, result: Err("gh down".to_string()) }).is_empty());
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Copy PR #10 URL failed: gh down"));
        press(&mut state, Msg::Copied { target, result: Err("no tmux".to_string()) });
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Copy PR #10 URL failed: no tmux"));
    }

    #[test]
    fn a_failed_open_shows_the_error_and_a_success_is_quiet() {
        let (mut state, _) = app();
        let target = GithubRef::Pr(PrNumber(10));
        assert!(press(&mut state, Msg::Opened { target, result: Ok(()) }).is_empty());
        assert!(state.toast.is_none());
        press(&mut state, Msg::Opened { target, result: Err("no browser".to_string()) });
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Open PR #10 failed: no browser"));
    }

    fn sends(cmds: &[Cmd]) -> Vec<(IssueNumber, &str)> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::Job(Job::Send { issue, text }) => Some((*issue, text.as_str())),
                _ => None,
            })
            .collect()
    }

    fn merges(cmds: &[Cmd]) -> Vec<MergeAsk> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::Job(Job::Merge(ask)) => Some(*ask),
                _ => None,
            })
            .collect()
    }

    fn toast(state: &State) -> String {
        state.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default()
    }

    const ASK: MergeAsk = MergeAsk { pr: PrNumber(10), new_session_ok: false, ready_ok: false };
    const ASK_FIX: MergeAsk = MergeAsk { pr: PrNumber(10), new_session_ok: true, ready_ok: false };

    fn ran(result: Result<MergeOutcome, String>) -> MergeRun {
        MergeRun { marked_ready: false, result }
    }

    fn ran_after_ready(result: Result<MergeOutcome, String>) -> MergeRun {
        MergeRun { marked_ready: true, result }
    }

    fn handoff(problem: MergeProblem) -> Msg {
        let mut fresh = pr(10, "fix/3-x", &[3]);
        fresh.mergeability = Mergeability::Conflict;
        Msg::Merged {
            ask: ASK,
            run: ran(Ok(MergeOutcome::Handoff { problem, method: MergeMethod::Rebase, pr: fresh })),
        }
    }

    fn with_ask(msg: Msg, ask: MergeAsk) -> Msg {
        match msg {
            Msg::Merged { run, .. } => Msg::Merged { ask, run },
            other => other,
        }
    }

    #[test]
    fn m_twice_starts_a_merge_job_for_the_session_pr() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, true))));
        press(&mut state, key('3'));
        assert!(key_bar(&state).contains(&("m", "merge")));
        assert!(press(&mut state, key('m')).is_empty(), "the first press only arms");
        assert_eq!(toast(&state), "Press m again to merge PR #10");
        let cmds = press(&mut state, key('m'));
        assert_eq!(merges(&cmds), [ASK]);
        assert!(sends(&cmds).is_empty(), "no agent turn for the merge itself");
        assert!(state.armed.is_none());
        assert!(press(&mut state, key('m')).is_empty(), "a third press arms again");
    }

    #[test]
    fn m_merges_without_a_session_from_the_pr_or_the_issue() {
        let (mut state, _) = app();
        assert!(key_bar(&state).contains(&("m", "merge")), "issue #3 has PR #10");
        press(&mut state, key('m'));
        assert_eq!(merges(&press(&mut state, key('m'))), [ASK]);

        press(&mut state, key('2'));
        press(&mut state, key('j'));
        press(&mut state, key('m'));
        assert_eq!(toast(&state), "Press m again to merge PR #11");
        assert_eq!(
            merges(&press(&mut state, key('m'))),
            [MergeAsk { pr: PrNumber(11), new_session_ok: false, ready_ok: false }]
        );
    }

    #[test]
    fn the_first_m_names_the_problem_and_the_agent_of_a_running_session() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        state.prs.items[0].mergeability = Mergeability::Conflict;
        press(&mut state, key('3'));
        press(&mut state, key('m'));
        assert_eq!(
            toast(&state),
            "PR #10 has a merge conflict with main. Press m again to ask #3's agent to fix and merge it."
        );
        assert_eq!(merges(&press(&mut state, key('m'))), [ASK], "the job decides, not the cached list");
    }

    #[test]
    fn without_a_running_session_the_first_m_offers_a_new_session() {
        let (mut state, _) = app();
        state.prs.items[0].mergeability = Mergeability::Conflict;
        press(&mut state, key('m'));
        assert_eq!(
            toast(&state),
            "PR #10 has a merge conflict with main. Press m again to start a session for #3 that fixes it."
        );
        assert_eq!(merges(&press(&mut state, key('m'))), [ASK_FIX]);
    }

    #[test]
    fn a_problem_found_after_the_user_agreed_starts_a_session_on_the_pr_branch() {
        let (mut state, _) = app();
        let cmds = press(&mut state, with_ask(handoff(MergeProblem::Conflict), ASK_FIX));
        let started = starts(&cmds);
        assert_eq!(started.len(), 1);
        let req = started[0];
        assert_eq!(
            (req.issue, req.branch.as_str(), req.mode, req.from),
            (IssueNumber(3), "fix/3-x", Mode::Implement, StartFrom::Origin)
        );
        assert_eq!(
            req.prompt,
            rules::fix_session_prompt(PrNumber(10), "main", MergeProblem::Conflict, MergeMethod::Rebase)
        );
        assert!(state.starting.contains(&IssueNumber(3)));
        assert_eq!((state.tab, state.selected.session), (Tab::Sessions, Some(IssueNumber(3))));
        assert_eq!(toast(&state), "PR #10 has a merge conflict with main. Starting a session for #3 to fix it.");
    }

    #[test]
    fn a_problem_the_list_did_not_show_needs_one_more_m_for_a_new_session() {
        let (mut state, _) = app();
        press(&mut state, key('m'));
        press(&mut state, key('m'));
        let cmds = press(&mut state, handoff(MergeProblem::Conflict));
        assert!(cmds.is_empty(), "no session before the user agrees");
        assert_eq!(
            toast(&state),
            "PR #10 has a merge conflict with main. Press m again to start a session for #3 that fixes it."
        );
        let started = press(&mut state, key('m'));
        assert_eq!(starts(&started).iter().map(|r| r.issue).collect::<Vec<_>>(), [IssueNumber(3)]);
        assert!(merges(&started).is_empty());
    }

    #[test]
    fn another_key_cancels_the_new_session() {
        let (mut state, _) = app();
        press(&mut state, handoff(MergeProblem::Conflict));
        press(&mut state, key('j'));
        press(&mut state, key('k'));
        let cmds = press(&mut state, key('m'));
        assert!(starts(&cmds).is_empty() && merges(&cmds).is_empty());
        assert_eq!(toast(&state), "Press m again to merge PR #10", "the m arms a fresh merge");
    }

    #[test]
    fn an_exited_session_on_the_pr_branch_gets_a_new_session() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        let cmds = press(&mut state, with_ask(handoff(MergeProblem::ChecksFailing), ASK_FIX));
        assert!(sends(&cmds).is_empty());
        assert_eq!(starts(&cmds).iter().map(|r| r.issue).collect::<Vec<_>>(), [IssueNumber(3)]);
    }

    #[test]
    fn a_problem_pr_with_no_issue_to_start_on_only_explains() {
        let (mut state, _) = app();
        state.prs.items[1].mergeability = Mergeability::Conflict;
        press(&mut state, key('2'));
        state.selected.pr = Some(PrNumber(11));
        assert!(press(&mut state, key('m')).is_empty());
        assert_eq!(
            toast(&state),
            "PR #11 has a merge conflict with main. PR #11 closes no issue, so dispatch can not start a session for it."
        );
        assert!(state.armed.is_none());
    }

    #[test]
    fn a_direct_merge_shows_the_method_and_refreshes_the_prs() {
        let (mut state, _) = app();
        let cmds = press(&mut state, Msg::Merged { ask: ASK, run: ran(Ok(MergeOutcome::Merged(MergeMethod::Squash))) });
        assert_eq!(toast(&state), "Merged PR #10 (squash)");
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
        assert!(sends(&cmds).is_empty());
        assert!(stops(&cmds).is_empty(), "no session on the branch");
    }

    fn stops(cmds: &[Cmd]) -> Vec<IssueNumber> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::Job(Job::Stop(n)) => Some(*n),
                _ => None,
            })
            .collect()
    }

    fn merged_squash() -> Msg {
        Msg::Merged { ask: ASK, run: ran(Ok(MergeOutcome::Merged(MergeMethod::Squash))) }
    }

    #[test]
    fn a_direct_merge_stops_the_live_session_on_the_pr_branch() {
        for waiting in [false, true] {
            let (mut state, _) = app();
            state.sessions.insert(IssueNumber(3), session(3, Some((true, waiting))));
            let cmds = press(&mut state, merged_squash());
            assert_eq!(stops(&cmds), [IssueNumber(3)]);
            assert_eq!(toast(&state), "Merged PR #10 (squash) and stopped #3's session");
            assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
        }
    }

    #[test]
    fn a_direct_merge_leaves_an_exited_or_unrelated_session_alone() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        state.sessions.insert(IssueNumber(2), session(2, Some((true, false))));
        let cmds = press(&mut state, merged_squash());
        assert!(stops(&cmds).is_empty(), "#3's window is gone and #2 is on another branch");
        assert_eq!(toast(&state), "Merged PR #10 (squash)");

        state.sessions.insert(IssueNumber(3), session(3, None));
        assert!(stops(&press(&mut state, merged_squash())).is_empty(), "no window");
    }

    #[test]
    fn a_queued_merge_keeps_the_session() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        let cmds = press(&mut state, Msg::Merged { ask: ASK, run: ran(Ok(MergeOutcome::Queued)) });
        assert!(stops(&cmds).is_empty(), "the PR can still drop out of the queue");
    }

    #[test]
    fn a_problem_pr_sends_the_running_agent_a_prompt_with_the_problem_and_method() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        let cmds = press(&mut state, handoff(MergeProblem::ChecksFailing));
        let prompt = rules::handoff_prompt(PrNumber(10), "main", MergeProblem::ChecksFailing, MergeMethod::Rebase);
        assert_eq!(sends(&cmds), [(IssueNumber(3), prompt.as_str())]);
        assert!(prompt.contains("failing checks") && prompt.contains("--rebase"));
        assert_eq!(toast(&state), "PR #10 has failing checks. Asking #3's agent to fix it.");
    }

    #[test]
    fn a_queued_merge_says_so() {
        let (mut state, _) = app();
        let cmds = press(&mut state, Msg::Merged { ask: ASK, run: ran(Ok(MergeOutcome::Queued)) });
        assert_eq!(toast(&state), "PR #10 is in the merge queue");
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))));
    }

    #[test]
    fn a_refused_merge_shows_a_toast_and_refreshes_and_a_failed_one_only_the_toast() {
        let (mut state, _) = app();
        let cmds = press(&mut state, Msg::Merged { ask: ASK, run: ran(Ok(MergeOutcome::Refused("wait".to_string()))) });
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))), "a stale list catches up");
        assert_eq!(toast(&state), "wait");
        let cmds = press(&mut state, Msg::Merged { ask: ASK, run: ran(Err("not allowed".to_string())) });
        assert!(cmds.is_empty());
        assert_eq!(toast(&state), "Merge PR #10 failed: not allowed");
    }

    const ASK_READY: MergeAsk = MergeAsk { pr: PrNumber(10), new_session_ok: false, ready_ok: true };

    #[test]
    fn m_on_a_draft_says_it_marks_it_ready_then_the_second_m_merges_with_consent() {
        let (mut state, _) = app();
        state.prs.items[0].state = PrState::Draft;
        assert!(key_bar(&state).contains(&("m", "merge")));
        assert!(press(&mut state, key('m')).is_empty());
        assert_eq!(toast(&state), "PR #10 is a draft. Press m again to mark it ready and merge it.");
        let cmds = press(&mut state, key('m'));
        assert_eq!(merges(&cmds), [ASK_READY]);
    }

    #[test]
    fn a_merge_that_marked_the_pr_ready_but_did_not_merge_says_so_and_refreshes() {
        let handoff_after_ready = || {
            let mut fresh = pr(10, "fix/3-x", &[3]);
            fresh.mergeability = Mergeability::Conflict;
            Ok(MergeOutcome::Handoff { problem: MergeProblem::Conflict, method: MergeMethod::Rebase, pr: fresh })
        };
        let outcomes = [
            (Ok(MergeOutcome::Refused("wait.".to_string())), "wait. PR #10 is now ready for review."),
            (
                handoff_after_ready(),
                "PR #10 has a merge conflict with main. Press m again to start a session for #3 that fixes it. PR #10 is now ready for review.",
            ),
            (Err("boom".to_string()), "Merge PR #10 failed: boom PR #10 is now ready for review."),
        ];
        for (result, text) in outcomes {
            let (mut state, _) = app();
            let cmds = press(&mut state, Msg::Merged { ask: ASK_READY, run: ran_after_ready(result) });
            assert_eq!(toast(&state), text);
            assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchPrs))), "{text}");
            assert!(cmds.iter().any(|c| matches!(c, Cmd::Job(Job::FetchIssues))), "{text}");
        }
    }

    #[test]
    fn a_merge_after_marking_ready_has_no_ready_note() {
        let (mut state, _) = app();
        let msg = Msg::Merged { ask: ASK_READY, run: ran_after_ready(Ok(MergeOutcome::Merged(MergeMethod::Squash))) };
        press(&mut state, msg);
        assert_eq!(toast(&state), "Merged PR #10 (squash)");
    }

    #[test]
    fn m_from_the_pr_row_reaches_the_same_pr() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        press(&mut state, key('2'));
        press(&mut state, key('m'));
        assert_eq!(merges(&press(&mut state, key('m'))), [ASK]);
    }

    #[test]
    fn another_key_between_the_two_m_presses_disarms() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        press(&mut state, key('3'));
        press(&mut state, key('m'));
        press(&mut state, key('r'));
        assert!(merges(&press(&mut state, key('m'))).is_empty());
    }

    #[test]
    fn m_explains_why_it_can_not_merge() {
        let (mut state, _) = app();
        press(&mut state, key('3'));
        press(&mut state, key('m'));
        assert_eq!(toast(&state), "Nothing is selected.");

        press(&mut state, key('1'));
        state.prs.items[0].state = PrState::Draft;
        press(&mut state, key('m'));
        assert_eq!(toast(&state), "PR #10 is a draft. Press m again to mark it ready and merge it.");
        assert!(key_bar(&state).contains(&("m", "merge")));

        state.prs.items.remove(0);
        for _ in 0..2 {
            assert!(press(&mut state, key('m')).is_empty());
        }
        assert_eq!(toast(&state), "#3 has no PR yet.");
    }

    #[test]
    fn a_sent_result_shows_a_toast() {
        let (mut state, _) = app();
        press(&mut state, Msg::Sent { issue: IssueNumber(3), result: Ok(()) });
        assert!(state.toast.as_ref().is_some_and(|t| t.text == "Asked #3's agent to fix and merge its PR"));
        press(&mut state, Msg::Sent { issue: IssueNumber(3), result: Err("gone".to_string()) });
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
        let (mut state, _) = app();
        let mut ev = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        ev.kind = KeyEventKind::Release;
        assert!(press(&mut state, Msg::Key(ev)).is_empty());
    }

    /// What a terminal sends for `S`.
    fn shift_s() -> Msg {
        Msg::Key(KeyEvent::new(KeyCode::Char('S'), KeyModifiers::SHIFT))
    }

    fn numbers(state: &State) -> Vec<u32> {
        visible_rows(state)
            .into_iter()
            .map(|k| match k {
                RowKey::Issue(n) | RowKey::Session(n) => n.0,
                RowKey::Pr(n) => n.0,
            })
            .collect()
    }

    /// `app()` plus: #1 needs you and has the most comments, #2 was updated last.
    fn sortable() -> State {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(1), session(1, Some((true, true))));
        state.issues.items[2].comment_count = 5;
        state.issues.items[1].updated_at = jiff::Timestamp::from_second(60).expect("in range");
        state
    }

    #[test]
    fn s_cycles_every_issue_mode_in_order_and_wraps() {
        let mut state = sortable();
        let mut seen = vec![(state.sort.label(Tab::Issues), numbers(&state))];
        for _ in 0..IssueSort::ALL.len() {
            assert!(press(&mut state, shift_s()).is_empty());
            seen.push((state.sort.label(Tab::Issues), numbers(&state)));
        }
        assert_eq!(
            seen,
            [
                ("attention", vec![1, 3, 2]),
                ("newest", vec![3, 2, 1]),
                ("oldest", vec![1, 2, 3]),
                ("recently updated", vec![2, 3, 1]),
                ("number", vec![1, 2, 3]),
                ("most comments", vec![1, 3, 2]),
                ("attention", vec![1, 3, 2]),
            ]
        );
    }

    #[test]
    fn each_tab_keeps_its_own_mode() {
        let mut state = sortable();
        press(&mut state, shift_s());
        press(&mut state, key('2'));
        assert_eq!(state.sort.label(Tab::Prs), "attention");
        assert_eq!(numbers(&state), [10, 11]);
        for _ in 0..4 {
            press(&mut state, shift_s());
        }
        assert_eq!(state.sort.label(Tab::Prs), "number");
        assert_eq!(numbers(&state), [10, 11]);
        press(&mut state, shift_s());
        assert_eq!(state.sort.label(Tab::Prs), "attention", "PRs have no comment mode");

        press(&mut state, key('3'));
        state.sessions.insert(IssueNumber(9), session(9, Some((true, false))));
        state.sessions.insert(IssueNumber(10), session(10, None));
        assert_eq!(numbers(&state), [1, 9, 10]);
        press(&mut state, shift_s());
        assert_eq!(state.sort.label(Tab::Sessions), "number");
        assert_eq!(numbers(&state), [1, 9, 10]);
        state.sessions.insert(IssueNumber(1), session(1, None));
        assert_eq!(numbers(&state), [1, 9, 10], "number mode ignores the state");
        press(&mut state, shift_s());
        assert_eq!(numbers(&state), [9, 1, 10]);

        press(&mut state, key('1'));
        assert_eq!(state.sort.label(Tab::Issues), "newest");
    }

    #[test]
    fn re_sorting_keeps_the_selected_row_even_when_it_was_only_the_first() {
        let mut state = sortable();
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
        press(&mut state, shift_s());
        assert_eq!(numbers(&state), [3, 2, 1]);
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(1))));
        press(&mut state, key('k'));
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
        press(&mut state, shift_s());
        assert_eq!(selected_row(&state), Some(RowKey::Issue(IssueNumber(2))));
    }

    #[test]
    fn the_sort_applies_to_the_filtered_rows_and_typing_s_in_the_filter_is_text() {
        let mut state = sortable();
        press(&mut state, key('/'));
        press(&mut state, key('j'));
        press(&mut state, shift_s());
        assert_eq!(state.filter, "jS");
        state.filter = "j".to_string();
        press(&mut state, code(KeyCode::Enter));
        assert_eq!(numbers(&state), [1, 3]);
        press(&mut state, shift_s());
        assert_eq!(numbers(&state), [3, 1]);
    }

    // ---- the settings menu ----

    fn menu(state: &State) -> &SettingsMenu {
        match &state.focus {
            Focus::Settings(m) => m,
            _ => panic!("the settings menu is not open"),
        }
    }

    fn open_menu_at(state: &mut State, setting: Setting) {
        press(state, key(','));
        while menu(state).at != setting {
            press(state, key('j'));
        }
    }

    fn type_text(state: &mut State, text: &str) {
        for c in text.chars() {
            press(state, key(c));
        }
    }

    fn clear_input(state: &mut State) {
        for _ in 0..40 {
            press(state, code(KeyCode::Backspace));
        }
    }

    fn saves(cmds: &[Cmd]) -> Vec<(Setting, &Config)> {
        cmds.iter()
            .filter_map(|c| match c {
                Cmd::Job(Job::SaveConfig { setting, config }) => Some((*setting, config)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn comma_opens_the_menu_and_the_key_bar_lists_it() {
        let (mut state, _) = app();
        assert!(key_bar(&state).contains(&(",", "settings")));
        assert!(press(&mut state, key(',')).is_empty());
        assert_eq!(menu(&state).at, SETTINGS[0]);
        assert!(menu(&state).edit.is_none());
        press(&mut state, key(','));
        assert!(matches!(state.focus, Focus::List), "comma closes it again");
        press(&mut state, key(','));
        press(&mut state, code(KeyCode::Esc));
        assert!(matches!(state.focus, Focus::List));
    }

    #[test]
    fn j_and_k_move_and_stop_at_the_ends() {
        let (mut state, _) = app();
        press(&mut state, key(','));
        press(&mut state, key('k'));
        assert_eq!(menu(&state).at, SETTINGS[0]);
        for _ in 0..SETTINGS.len() + 2 {
            press(&mut state, key('j'));
        }
        assert_eq!(menu(&state).at, SETTINGS[SETTINGS.len() - 1], "no wrap");
        press(&mut state, code(KeyCode::Up));
        assert_eq!(menu(&state).at, SETTINGS[SETTINGS.len() - 2]);
        press(&mut state, code(KeyCode::Down));
        assert_eq!(menu(&state).at, SETTINGS[SETTINGS.len() - 1]);
    }

    #[test]
    fn enter_toggles_detect_waiting_and_saves_it() {
        let (mut state, _) = app();
        press(&mut state, key(','));
        let cmds = press(&mut state, code(KeyCode::Enter));
        assert!(!state.config.detect_waiting);
        let [(Setting::DetectWaiting, saved)] = saves(&cmds)[..] else { panic!("one save: {}", cmds.len()) };
        assert!(!saved.detect_waiting);
        assert!(menu(&state).edit.is_none());
        let cmds = press(&mut state, code(KeyCode::Enter));
        assert!(state.config.detect_waiting);
        assert!(saves(&cmds)[0].1.detect_waiting);
    }

    #[test]
    fn enter_cycles_the_icons_and_saves_them() {
        let (mut state, _) = app();
        open_menu_at(&mut state, Setting::Icons);
        let cmds = press(&mut state, code(KeyCode::Enter));
        assert_eq!(state.config.icons, IconSet::NerdFont);
        assert_eq!(saves(&cmds)[0].0, Setting::Icons);
        press(&mut state, code(KeyCode::Enter));
        assert_eq!(state.config.icons, IconSet::Unicode);
    }

    #[test]
    fn a_valid_split_applies_now_and_saves() {
        let (mut state, _) = app();
        open_menu_at(&mut state, Setting::Split);
        assert!(press(&mut state, code(KeyCode::Enter)).is_empty(), "enter opens the editor");
        assert_eq!(menu(&state).edit.as_ref().map(|e| e.input.lines().join("")), Some("42".to_string()));
        clear_input(&mut state);
        type_text(&mut state, "60");
        let cmds = press(&mut state, code(KeyCode::Enter));
        assert_eq!(state.config.split, Split::from_percent(60).expect("in range"));
        assert_eq!(state.split, state.config.split, "the live border moves");
        assert_eq!(saves(&cmds)[0].0, Setting::Split);
        assert!(menu(&state).edit.is_none());
    }

    #[test]
    fn an_invalid_split_shows_an_error_and_keeps_editing() {
        for bad in ["95", "abc", ""] {
            let (mut state, _) = app();
            open_menu_at(&mut state, Setting::Split);
            press(&mut state, code(KeyCode::Enter));
            clear_input(&mut state);
            type_text(&mut state, bad);
            let cmds = press(&mut state, code(KeyCode::Enter));
            assert!(cmds.is_empty(), "{bad:?}");
            let edit = menu(&state).edit.as_ref().expect("still editing");
            assert!(edit.error.is_some(), "{bad:?}");
            assert_eq!(state.config.split, Split::DEFAULT);
            assert_eq!(state.split, Split::DEFAULT);
            type_text(&mut state, "5");
            assert!(menu(&state).edit.as_ref().is_some_and(|e| e.error.is_none()), "typing clears the error");
        }
    }

    #[test]
    fn the_split_error_says_what_the_file_says() {
        let (mut state, _) = app();
        open_menu_at(&mut state, Setting::Split);
        press(&mut state, code(KeyCode::Enter));
        clear_input(&mut state);
        type_text(&mut state, "95");
        press(&mut state, code(KeyCode::Enter));
        let error = menu(&state).edit.as_ref().and_then(|e| e.error.clone());
        assert_eq!(error, Some("invalid config: split 95 is not a percent from 10 to 90".to_string()));
    }

    #[test]
    fn a_claude_command_edit_applies_and_saves() {
        let (mut state, _) = app();
        open_menu_at(&mut state, Setting::ClaudeCommand);
        press(&mut state, code(KeyCode::Enter));
        type_text(&mut state, " --model 'big one'");
        let cmds = press(&mut state, code(KeyCode::Enter));
        assert_eq!(state.config.claude_command.argv(&[]), ["claude", "--model", "big one"]);
        assert_eq!(saves(&cmds)[0].0, Setting::ClaudeCommand);
    }

    #[test]
    fn a_blank_claude_command_is_refused() {
        let (mut state, _) = app();
        open_menu_at(&mut state, Setting::ClaudeCommand);
        press(&mut state, code(KeyCode::Enter));
        clear_input(&mut state);
        assert!(press(&mut state, code(KeyCode::Enter)).is_empty());
        assert!(menu(&state).edit.as_ref().is_some_and(|e| e.error.is_some()));
        assert_eq!(state.config, Config::default());
    }

    #[test]
    fn esc_while_editing_cancels_the_edit_only() {
        let (mut state, _) = app();
        open_menu_at(&mut state, Setting::Split);
        press(&mut state, code(KeyCode::Enter));
        type_text(&mut state, "7");
        press(&mut state, code(KeyCode::Esc));
        assert!(menu(&state).edit.is_none(), "the menu stays open");
        assert_eq!(menu(&state).at, Setting::Split);
        assert_eq!(state.config.split, Split::DEFAULT);
        press(&mut state, code(KeyCode::Esc));
        assert!(matches!(state.focus, Focus::List));
    }

    #[test]
    fn keys_in_the_editor_do_not_move_or_close_the_menu() {
        let (mut state, _) = app();
        open_menu_at(&mut state, Setting::ClaudeCommand);
        press(&mut state, code(KeyCode::Enter));
        type_text(&mut state, "j,q");
        assert_eq!(menu(&state).at, Setting::ClaudeCommand);
        assert_eq!(menu(&state).edit.as_ref().map(|e| e.input.lines().join("")), Some("claudej,q".to_string()));
    }

    #[test]
    fn a_new_start_uses_the_changed_settings() {
        let (mut state, _) = app();
        press(&mut state, key(','));
        press(&mut state, code(KeyCode::Enter));
        press(&mut state, code(KeyCode::Esc));
        press(&mut state, key('s'));
        let cmds = press(&mut state, ctrl('s'));
        let [Cmd::Job(Job::Start(_, launch))] = cmds.as_slice() else { panic!("one start") };
        assert!(!launch.detect_waiting, "the toggle reaches the next start");
        assert_eq!(*launch, Launch::from_config(&state.config));
    }

    #[test]
    fn a_resume_uses_the_changed_command() {
        let (mut state, _) = app();
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        open_menu_at(&mut state, Setting::ClaudeCommand);
        press(&mut state, code(KeyCode::Enter));
        type_text(&mut state, " --x");
        press(&mut state, code(KeyCode::Enter));
        press(&mut state, code(KeyCode::Esc));
        press(&mut state, key('3'));
        let cmds = press(&mut state, key('a'));
        let [Cmd::Job(Job::Resume(_, launch))] = cmds.as_slice() else { panic!("one resume") };
        assert_eq!(launch.claude.words(), "claude --x");
    }

    #[test]
    fn the_split_reset_key_returns_to_the_saved_split() {
        let (mut state, _) = app();
        open_menu_at(&mut state, Setting::Split);
        press(&mut state, code(KeyCode::Enter));
        clear_input(&mut state);
        type_text(&mut state, "70");
        press(&mut state, code(KeyCode::Enter));
        press(&mut state, code(KeyCode::Esc));
        state.split = Split::DEFAULT;
        press(&mut state, key('='));
        assert_eq!(state.split, Split::from_percent(70).expect("in range"));
    }

    #[test]
    fn a_failed_save_toasts_and_a_good_one_does_not() {
        let (mut state, _) = app();
        assert!(press(&mut state, Msg::Saved(Ok(()))).is_empty());
        assert!(state.toast.is_none());
        press(&mut state, Msg::Saved(Err("disk full".to_string())));
        assert_eq!(state.toast.as_ref().map(|t| t.text.as_str()), Some("Saving settings failed: disk full"));
    }
}
