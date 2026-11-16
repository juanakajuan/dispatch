//! UI state and the reducer. `update` is the only writer of `State`; it does
//! no I/O and returns `Cmd`s for the shell to run. Every key, click, tick, and
//! worker result is a `Msg`, so each behaviour is a unit test:
//! "given this state, this msg -> this state and these cmds".

use crate::config::{Change, Config, SETTINGS, Setting};
use crate::domain::{BranchName, GithubRef, Issue, IssueNumber, MergeMethod, Mode, PrNumber, PrState, PullRequest};
use crate::merge::{MergeOutcome, MergeRun};
use crate::rules::{self, MergePlan, MergeProblem};
use crate::session::request::{Launch, StartFrom, StartRequest};
use crate::session::{Session, SessionState, Sessions, StartOutcome, WindowId};
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

/// The section a row sits in. Lower `rank` first.
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
    /// The keybinds overlay, scrolled down by `scroll` lines.
    Keybinds {
        scroll: u16,
    },
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
pub struct Inflight {
    pub issues: bool,
    pub prs: bool,
    pub sessions: bool,
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
}

/// A message for the key bar. `update` has no clock for key presses, so the
/// next tick stamps `shown_at`, and a later tick removes the toast.
pub struct Toast {
    pub text: String,
    pub shown_at: Option<Instant>,
}

/// Clickable regions from the last frame. Written by the shell after each
/// draw (`ui::draw` returns it); read by `update` on mouse events.
#[derive(Default)]
pub struct HitMap {
    pub targets: Vec<(ratatui::layout::Rect, HitTarget)>,
    /// How far the keybinds overlay can scroll at the last drawn size.
    pub keybinds_max_scroll: u16,
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
}

/// Facts about the repo that the view shows, resolved once at startup.
#[derive(Clone, Debug)]
#[cfg_attr(test, derive(Default))]
pub struct Context {
    pub repo_name: String,
    pub open_with: OpenWith,
    pub start_preview: crate::session::StartPreview,
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
    pub toast: Option<Toast>,
    pub inflight: Inflight,
    pub due: Due,
    pub hits: HitMap,
    pub last_click: Option<(HitTarget, Instant)>,
    /// Set by an `m` that waits for one more `m` on the same PR. Any other
    /// key clears it.
    pub armed: Option<Armed>,
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
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const TOAST_FOR: Duration = Duration::from_secs(5);
const GITHUB_COOLDOWN: Duration = Duration::from_secs(5);

pub fn init(now: Instant, config: Config, context: Context) -> (State, Vec<Cmd>) {
    let state = State {
        context,
        tab: Tab::Issues,
        issues: Feed { items: Vec::new(), status: FeedStatus::Loading },
        prs: Feed { items: Vec::new(), status: FeedStatus::Loading },
        sessions: Sessions::new(),
        starting: BTreeSet::new(),
        selected: Selected::default(),
        sort: Sorts::default(),
        filter: String::new(),
        focus: Focus::List,
        toast: None,
        inflight: Inflight { issues: true, prs: true, sessions: true },
        due: Due {
            github: now + config.github_refresh,
            github_soon: false,
            github_sent: now,
            sessions: now + SESSIONS_EVERY,
        },
        hits: HitMap::default(),
        last_click: None,
        armed: None,
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
    Settings,
    Keybinds,
    Quit,
}

/// The heading a binding shows under in the keybinds menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyGroup {
    Navigation,
    Sessions,
    PullRequests,
    View,
    App,
}

impl KeyGroup {
    /// The menu order.
    pub const ALL: [Self; 5] = [Self::Navigation, Self::Sessions, Self::PullRequests, Self::View, Self::App];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Navigation => "Navigation",
            Self::Sessions => "Sessions",
            Self::PullRequests => "Pull requests",
            Self::View => "View",
            Self::App => "App",
        }
    }
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
    pub group: KeyGroup,
    /// What the keybinds menu says the keys do.
    pub desc: &'static str,
    pub bar: &'static [BarEntry],
}

impl Key {
    /// How the keybinds menu writes the key: `j`, `down`, `ctrl+c`.
    pub fn label(self) -> String {
        match self {
            Self::Char(c) => c.to_string(),
            Self::Ctrl(c) => format!("ctrl+{c}"),
            Self::Code(code) => code.to_string().to_lowercase(),
        }
    }
}

const fn bar(keys: &'static str, desc: &'static str, when: When) -> BarEntry {
    BarEntry { keys, desc, when }
}

/// The one table behind key handling, the key bar, and the keybinds menu.
pub static BINDINGS: &[Binding] = &[
    // First: the key bar keeps this order and a narrow terminal clips its end.
    Binding {
        keys: &[Key::Char('?')],
        action: Action::Keybinds,
        group: KeyGroup::App,
        desc: "show all keys",
        bar: &[bar("?", "keys", When::Always)],
    },
    Binding {
        keys: &[Key::Char('j'), Key::Code(KeyCode::Down)],
        action: Action::Down,
        group: KeyGroup::Navigation,
        desc: "move down",
        bar: &[],
    },
    Binding {
        keys: &[Key::Char('k'), Key::Code(KeyCode::Up)],
        action: Action::Up,
        group: KeyGroup::Navigation,
        desc: "move up",
        bar: &[],
    },
    Binding {
        keys: &[Key::Char('1')],
        action: Action::GoTab(Tab::Issues),
        group: KeyGroup::Navigation,
        desc: "go to the issues tab",
        bar: &[],
    },
    Binding {
        keys: &[Key::Char('2')],
        action: Action::GoTab(Tab::Prs),
        group: KeyGroup::Navigation,
        desc: "go to the pull requests tab",
        bar: &[],
    },
    Binding {
        keys: &[Key::Char('3')],
        action: Action::GoTab(Tab::Sessions),
        group: KeyGroup::Navigation,
        desc: "go to the sessions tab",
        bar: &[],
    },
    Binding {
        keys: &[Key::Char('s')],
        action: Action::Start,
        group: KeyGroup::Sessions,
        desc: "start a session for the issue",
        bar: &[bar("s", "start session", When::OnIssues)],
    },
    Binding {
        keys: &[Key::Code(KeyCode::Enter)],
        action: Action::Enter,
        group: KeyGroup::Navigation,
        desc: "start, go to the PR's issue, or attach",
        bar: &[bar("enter", "go to issue", When::OnPrs), bar("enter", "attach", When::OnSessions)],
    },
    Binding {
        keys: &[Key::Char('a')],
        action: Action::Attach,
        group: KeyGroup::Sessions,
        desc: "attach to the session, or resume it",
        bar: &[bar("a", "attach", When::SessionLive), bar("a", "resume", When::SessionExited)],
    },
    Binding {
        keys: &[Key::Char('x')],
        action: Action::Stop,
        group: KeyGroup::Sessions,
        desc: "stop the session",
        bar: &[bar("x", "stop", When::SessionLive)],
    },
    Binding {
        keys: &[Key::Char('m')],
        action: Action::Merge,
        group: KeyGroup::PullRequests,
        desc: "merge the PR (press twice)",
        bar: &[bar("m", "merge", When::Mergeable)],
    },
    Binding {
        keys: &[Key::Char('o')],
        action: Action::Open,
        group: KeyGroup::PullRequests,
        desc: "open on GitHub, or copy the URL",
        bar: &[
            bar("o", "open on GitHub", When::OpenWith(OpenWith::Browser)),
            bar("o", "copy URL", When::OpenWith(OpenWith::Clipboard)),
        ],
    },
    Binding {
        keys: &[Key::Char('r')],
        action: Action::Refresh,
        group: KeyGroup::App,
        desc: "load issues and PRs from GitHub now",
        bar: &[],
    },
    Binding {
        keys: &[Key::Char('/')],
        action: Action::Filter,
        group: KeyGroup::View,
        desc: "filter the list",
        bar: &[],
    },
    Binding {
        keys: &[Key::Code(KeyCode::Esc)],
        action: Action::ClearFilter,
        group: KeyGroup::View,
        desc: "clear the filter",
        bar: &[],
    },
    // Not in the key bar: the footer shows `S sort: <mode>`.
    Binding {
        keys: &[Key::Char('S')],
        action: Action::Sort,
        group: KeyGroup::View,
        desc: "change the sort order",
        bar: &[],
    },
    Binding {
        keys: &[Key::Char('q'), Key::Ctrl('c')],
        action: Action::Quit,
        group: KeyGroup::App,
        desc: "quit",
        bar: &[bar("q", "quit", When::Always)],
    },
    Binding {
        keys: &[Key::Char(',')],
        action: Action::Settings,
        group: KeyGroup::App,
        desc: "open the settings menu",
        bar: &[],
    },
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
            Focus::Keybinds { .. } => keybinds_key(state, &ev),
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
        Action::GoTab(tab) => state.tab = tab,
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
        Action::Settings => state.focus = Focus::Settings(Box::new(SettingsMenu { at: SETTINGS[0], edit: None })),
        Action::Keybinds => state.focus = Focus::Keybinds { scroll: 0 },
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
}

fn on_mouse(state: &mut State, ev: MouseEvent, now: Instant) -> Vec<Cmd> {
    if !matches!(state.focus, Focus::List) {
        return Vec::new();
    }
    match ev.kind {
        MouseEventKind::ScrollDown => move_selection(state, Move::Down),
        MouseEventKind::ScrollUp => move_selection(state, Move::Up),
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(target) = state.hits.at(ev.column, ev.row) else { return Vec::new() };
            let double = state.last_click.is_some_and(|(t, at)| t == target && now.duration_since(at) <= DOUBLE_CLICK);
            state.last_click = if double { None } else { Some((target, now)) };
            match target {
                HitTarget::Tab(tab) => state.tab = tab,
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

// ---- the keybinds menu -------------------------------------------------------

fn keybinds_key(state: &mut State, ev: &KeyEvent) -> Vec<Cmd> {
    let Focus::Keybinds { scroll } = &mut state.focus else { return Vec::new() };
    match key_of(ev) {
        Some(Key::Char('j') | Key::Code(KeyCode::Down)) => {
            *scroll = scroll.saturating_add(1).min(state.hits.keybinds_max_scroll);
        }
        Some(Key::Char('k') | Key::Code(KeyCode::Up)) => *scroll = scroll.saturating_sub(1),
        Some(Key::Code(KeyCode::Esc) | Key::Char('?')) => state.focus = Focus::List,
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

/// `state.config` already holds the new value. Save it.
fn changed(state: &State, setting: Setting) -> Vec<Cmd> {
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
    rows.sort_by_key(|k| group_of(state, *k).rank);
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
mod tests;
