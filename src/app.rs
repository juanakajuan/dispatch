//! UI state and the reducer. `update` is the only writer of `State`; it does
//! no I/O and returns `Cmd`s for the shell to run. Every key, click, tick, and
//! worker result is a `Msg`, so each behaviour is a unit test:
//! "given this state, this msg -> this state and these cmds".

use crate::config::Config;
use crate::domain::{BranchName, Issue, IssueNumber, Mode, PrNumber, PullRequest, SkillChoice};
use crate::rules;
use crate::session::{Session, SessionState, Sessions, StartOutcome, StartRequest, WindowId};
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

const FIELDS: [DialogField; 4] = [DialogField::Branch, DialogField::Skill, DialogField::Mode, DialogField::Prompt];

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

/// Clickable regions from the last frame. Written by the shell after each
/// draw (`ui::draw` returns it); read by `update` on mouse events.
#[derive(Default)]
pub struct HitMap {
    pub targets: Vec<(ratatui::layout::Rect, HitTarget)>,
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
    Started { issue: IssueNumber, result: Result<StartOutcome, String> },
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
}

pub const SESSIONS_EVERY: Duration = Duration::from_secs(2);
pub const TAIL_EVERY: Duration = Duration::from_secs(1);
pub const TAIL_LINES: u16 = 60;
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const TOAST_FOR: Duration = Duration::from_secs(5);

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
        due: Due { github: now + config.github_refresh, sessions: now + SESSIONS_EVERY, tail: None },
        hits: HitMap::default(),
        last_click: None,
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
    Refresh,
    Filter,
    ClearFilter,
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
    Binding { keys: &[Key::Char('r')], action: Action::Refresh, bar: &[bar("r", "refresh", When::Always)] },
    Binding { keys: &[Key::Char('/')], action: Action::Filter, bar: &[bar("/", "filter", When::Always)] },
    Binding { keys: &[Key::Code(KeyCode::Esc)], action: Action::ClearFilter, bar: &[] },
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
            Focus::List => match action_for(&ev) {
                Some(action) => act(state, action, config),
                None => Vec::new(),
            },
        },
        Msg::Mouse(ev, now) => on_mouse(state, &ev, now, config),
        Msg::Tick(now) => tick(state, now, config),
        Msg::Issues(result) => {
            state.inflight.issues = false;
            apply_feed(&mut state.issues, result);
            Vec::new()
        }
        Msg::Prs(result) => {
            state.inflight.prs = false;
            apply_feed(&mut state.prs, result);
            Vec::new()
        }
        Msg::Sessions(result) => {
            state.inflight.sessions = false;
            match result {
                Ok(sessions) => state.sessions = sessions,
                Err(e) => say(state, format!("sessions: {e}")),
            }
            Vec::new()
        }
        Msg::Tail { window, result } => {
            state.inflight.tail = false;
            let in_view = current_session(state).and_then(|s| s.window.as_ref()).is_some_and(|w| w.id == window);
            if in_view {
                if let Ok(lines) = result {
                    state.tail = Some(Tail { window, lines });
                }
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
                    say(state, text);
                }
                Err(e) => say(state, format!("Start #{} failed: {e}", issue.0)),
            }
            observe_now(state)
        }
        Msg::Stopped { issue, result } => {
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
        Msg::Attached(result) => {
            if let Err(e) = result {
                say(state, format!("attach failed: {e}"));
            }
            observe_now(state)
        }
    }
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

fn apply_feed<T>(feed: &mut Feed<T>, result: Result<Vec<T>, String>) {
    match result {
        Ok(items) => {
            feed.items = items;
            feed.status = FeedStatus::Fresh;
        }
        Err(e) => feed.status = FeedStatus::Failed(e),
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
    if now >= state.due.github {
        state.due.github = now + config.github_refresh;
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

fn act(state: &mut State, action: Action, config: &Config) -> Vec<Cmd> {
    match action {
        Action::GoTab(tab) => {
            state.tab = tab;
            state.due.tail = None;
        }
        Action::Down => move_selection(state, Move::Down),
        Action::Up => move_selection(state, Move::Up),
        Action::Start => return start_key(state, config),
        Action::Enter => match state.tab {
            Tab::Issues => return start_key(state, config),
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
        Action::Refresh => {
            let mut cmds = Vec::new();
            refresh_github(state, &mut cmds);
            cmds.extend(observe_now(state));
            return cmds;
        }
        Action::Filter => state.focus = Focus::Filter,
        Action::ClearFilter => state.filter.clear(),
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

/// `s` or Enter on an issue, or a double-click on one.
fn start_key(state: &mut State, config: &Config) -> Vec<Cmd> {
    let Some(RowKey::Issue(n)) = selected_row(state).filter(|_| state.tab == Tab::Issues) else {
        return Vec::new();
    };
    start_issue(state, n, config);
    Vec::new()
}

fn start_issue(state: &mut State, n: IssueNumber, config: &Config) {
    if state.sessions.contains_key(&n) {
        goto_session(state, n);
    } else if state.starting.contains(&n) {
        say(state, format!("#{} is already starting", n.0));
    } else if let Some(issue) = state.issues.items.iter().find(|i| i.number == n) {
        state.focus = Focus::Dialog(Box::new(open_dialog(issue, config)));
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

fn on_mouse(state: &mut State, ev: &MouseEvent, now: Instant, config: &Config) -> Vec<Cmd> {
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
                HitTarget::Tab(tab) => {
                    state.tab = tab;
                    state.due.tail = None;
                }
                HitTarget::Row(key) => {
                    select(state, key);
                    if let (true, RowKey::Issue(n)) = (double, key) {
                        start_issue(state, n, config);
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

fn open_dialog(issue: &Issue, config: &Config) -> StartDialog {
    let defaults = rules::start_defaults(issue, config);
    let mut choices: Vec<SkillChoice> = vec![None];
    choices.extend(config.skills.iter().cloned().map(Some));
    let skill = choices.iter().position(|c| *c == defaults.skill).unwrap_or(0);
    let mut branch = tui_textarea::TextArea::new(vec![defaults.branch.as_str().to_string()]);
    branch.move_cursor(tui_textarea::CursorMove::End);
    let prompt = tui_textarea::TextArea::new(defaults.prompt.split('\n').map(str::to_string).collect());
    StartDialog {
        issue: issue.number,
        title: issue.title.clone(),
        branch,
        skill,
        choices,
        skill_reason: defaults.skill_reason,
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
        Some(Key::Code(KeyCode::BackTab)) => dialog.field = step_field(dialog.field, 3),
        Some(Key::Code(KeyCode::Down)) if dialog.field != DialogField::Prompt => dialog.field = step_field(dialog.field, 1),
        Some(Key::Code(KeyCode::Up)) if dialog.field != DialogField::Prompt => dialog.field = step_field(dialog.field, 3),
        Some(Key::Code(code @ (KeyCode::Left | KeyCode::Right))) if dialog.field == DialogField::Skill => {
            let n = dialog.choices.len();
            dialog.skill = if code == KeyCode::Right { (dialog.skill + 1) % n } else { (dialog.skill + n - 1) % n };
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
    let req = StartRequest {
        issue: dialog.issue,
        branch,
        skill: dialog.choices.get(dialog.skill).cloned().flatten(),
        mode: dialog.mode,
        prompt,
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

/// A PR whose head branch equals the session branch.
pub fn pr_for_session<'a>(state: &'a State, session: &Session) -> Option<&'a PullRequest> {
    state.prs.items.iter().find(|p| p.head == session.branch.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SkillName, WorktreePath};
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
            skill: SkillName::parse("pstack:tdd").ok(),
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
            status: FeedStatus::Fresh,
        };
        state.prs = Feed { items: vec![pr(10, "fix/3-x", &[3]), pr(11, "other", &[])], status: FeedStatus::Fresh };
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
        assert_eq!(d.choices[d.skill].as_ref().map(SkillName::as_str), Some("pstack:tdd"));
        assert_eq!(d.skill_reason.as_deref(), Some("bug"));
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
        for _ in 0..3 {
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
    fn dialog_skill_and_mode_cycle_and_reach_the_request() {
        let (mut state, config, _) = app();
        press(&mut state, &config, key('s'));
        press(&mut state, &config, code(KeyCode::Tab));
        press(&mut state, &config, code(KeyCode::Left));
        assert_eq!(dialog(&state).skill, 0, "tdd is first after automatic");
        press(&mut state, &config, code(KeyCode::Tab));
        press(&mut state, &config, code(KeyCode::Right));
        assert_eq!(dialog(&state).mode, Mode::PlanFirst);
        let cmds = press(&mut state, &config, ctrl('s'));
        let reqs = starts(&cmds);
        assert_eq!((reqs[0].skill.clone(), reqs[0].mode), (None, Mode::PlanFirst));
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
    fn issues_error_keeps_the_old_items() {
        let (mut state, config, _) = app();
        state.inflight.issues = true;
        press(&mut state, &config, Msg::Issues(Err("offline".to_string())));
        assert_eq!(state.issues.items.len(), 3);
        assert!(matches!(&state.issues.status, FeedStatus::Failed(m) if m == "offline"));
        assert!(!state.inflight.issues);
        press(&mut state, &config, Msg::Issues(Ok(vec![issue(9, "new", &[])])));
        assert_eq!(state.issues.items.len(), 1);
        assert!(matches!(state.issues.status, FeedStatus::Fresh));
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

    #[test]
    fn key_bar_follows_the_context() {
        let (mut state, config, _) = app();
        let names = |s: &State| key_bar(s).iter().map(|(k, d)| format!("{k} {d}")).collect::<Vec<_>>();
        assert!(names(&state).contains(&"s start session".to_string()));
        assert!(!names(&state).iter().any(|k| k.starts_with("x ")));
        state.sessions.insert(IssueNumber(3), session(3, Some((true, false))));
        assert!(names(&state).contains(&"a attach".to_string()));
        assert!(names(&state).contains(&"x stop".to_string()));
        state.sessions.insert(IssueNumber(3), session(3, Some((false, false))));
        assert!(names(&state).contains(&"a resume".to_string()));
        press(&mut state, &config, key('2'));
        assert!(names(&state).contains(&"enter go to issue".to_string()));
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
