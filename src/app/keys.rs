use super::batch::{mark_key, markable};
use super::diff::diff_key;
use super::feed::{observe_now, refresh_github};
use super::list::{
    Move, Tab, closed_issue_of_pr, current_session, go_to_issue_of_pr, move_selection, select, selected_row,
};
use super::menus::SettingsMenu;
use super::merge::{Armed, merge_key, merge_target};
use super::message::open_message;
use super::msg::Cmd;
use super::open::open_key;
use super::remove::{clean_key, has_merged_sessions, removable, remove_key};
use super::sessions::{attach_key, can_start, shell_key, start_key, stop_key};
use super::summary::{has_transcript_summary, summary_key};
use super::{Focus, OpenWith, State};
use crate::config::settings::SETTINGS;
use crate::session::SessionState;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
    Mark,
    Enter,
    Attach,
    Stop,
    Remove,
    Clean,
    Open,
    Merge,
    Refresh,
    Filter,
    ClearFilter,
    Sort,
    Settings,
    Keybinds,
    Summary,
    Diff,
    Shell,
    Message,
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
    CanStart,
    Markable,
    PrClosesIssue,
    SessionLive,
    SessionExited,
    HasTranscriptSummary,
    /// The selection has a session.
    HasSession,
    HasDiff,
    /// The session exited, or its PR is merged.
    Removable,
    HasMergedSessions,
    /// The selection has an open or draft PR.
    Mergeable,
    OpenWith(OpenWith),
}

pub struct BarEntry {
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
    /// The key press that `key_of` reads back as this key.
    pub fn event(self) -> KeyEvent {
        match self {
            Self::Char(c) => KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            Self::Ctrl(c) => KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
            Self::Code(code) => KeyEvent::new(code, KeyModifiers::NONE),
        }
    }

    /// How the key bar and the keybinds menu write the key: `j`, `down`, `ctrl+c`.
    pub fn label(self) -> String {
        match self {
            Self::Char(' ') => "space".to_string(),
            Self::Char(c) => c.to_string(),
            Self::Ctrl(c) => format!("ctrl+{c}"),
            Self::Code(code) => code.to_string().to_lowercase(),
        }
    }
}

/// Keys that do one thing, written as one hint: `j/down`.
pub fn labels(keys: &[Key]) -> String {
    keys.iter().map(|k| k.label()).collect::<Vec<_>>().join("/")
}

pub(super) const fn bar(desc: &'static str, when: When) -> BarEntry {
    BarEntry { desc, when }
}

/// The one table behind key handling, the key bar, and the keybinds menu.
pub static BINDINGS: &[Binding] = &[
    // First: the key bar keeps this order and a narrow terminal clips its end.
    Binding {
        keys: &[Key::Char('?')],
        action: Action::Keybinds,
        group: KeyGroup::App,
        desc: "show all keys",
        bar: &[bar("show keys", When::Always)],
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
        desc: "start a session for the issue or PR",
        bar: &[bar("start session", When::CanStart)],
    },
    Binding {
        keys: &[Key::Char(' ')],
        action: Action::Mark,
        group: KeyGroup::Sessions,
        desc: "mark the issue to start several at once",
        bar: &[bar("mark", When::Markable)],
    },
    Binding {
        keys: &[Key::Code(KeyCode::Enter)],
        action: Action::Enter,
        group: KeyGroup::Navigation,
        desc: "start, go to the PR's issue, or attach",
        bar: &[bar("go to issue", When::PrClosesIssue)],
    },
    Binding {
        keys: &[Key::Char('a')],
        action: Action::Attach,
        group: KeyGroup::Sessions,
        desc: "attach to the session, or resume it",
        bar: &[bar("attach", When::SessionLive), bar("resume", When::SessionExited)],
    },
    Binding {
        keys: &[Key::Char('x')],
        action: Action::Stop,
        group: KeyGroup::Sessions,
        desc: "stop the session",
        bar: &[bar("stop", When::SessionLive)],
    },
    Binding {
        keys: &[Key::Char('t')],
        action: Action::Message,
        group: KeyGroup::Sessions,
        desc: "send a message to the session",
        bar: &[bar("send message", When::SessionLive)],
    },
    Binding {
        keys: &[Key::Char('i')],
        action: Action::Summary,
        group: KeyGroup::Sessions,
        desc: "show all of what Claude last said",
        bar: &[bar("read summary", When::HasTranscriptSummary)],
    },
    Binding {
        keys: &[Key::Char('d')],
        action: Action::Diff,
        group: KeyGroup::Sessions,
        desc: "show the diff of the session's worktree",
        bar: &[bar("show diff", When::HasDiff)],
    },
    Binding {
        keys: &[Key::Char('!')],
        action: Action::Shell,
        group: KeyGroup::Sessions,
        desc: "open a shell in the session's worktree",
        bar: &[bar("open shell", When::HasSession)],
    },
    Binding {
        keys: &[Key::Char('D')],
        action: Action::Remove,
        group: KeyGroup::Sessions,
        desc: "remove the worktree and branch (press twice)",
        bar: &[bar("remove", When::Removable)],
    },
    Binding {
        keys: &[Key::Char('c')],
        action: Action::Clean,
        group: KeyGroup::Sessions,
        desc: "remove all merged sessions (press twice)",
        bar: &[bar("clean merged", When::HasMergedSessions)],
    },
    Binding {
        keys: &[Key::Char('m')],
        action: Action::Merge,
        group: KeyGroup::PullRequests,
        desc: "merge the PR (press twice)",
        bar: &[bar("merge", When::Mergeable)],
    },
    Binding {
        keys: &[Key::Char('o')],
        action: Action::Open,
        group: KeyGroup::PullRequests,
        desc: "open on GitHub, or copy the URL",
        bar: &[
            bar("open on GitHub", When::OpenWith(OpenWith::Browser)),
            bar("copy URL", When::OpenWith(OpenWith::Clipboard)),
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
        bar: &[bar("quit", When::Always)],
    },
    Binding {
        keys: &[Key::Char(',')],
        action: Action::Settings,
        group: KeyGroup::App,
        desc: "open the settings menu",
        bar: &[],
    },
];

/// One footer hint, shown as `<keys> to <desc>`: `keys` is written with
/// `Key::label`, and `key` is the key a click on it presses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub keys: String,
    pub desc: &'static str,
    pub key: Key,
}

fn hint(key: Key, desc: &'static str) -> Hint {
    Hint { keys: key.label(), desc, key }
}

/// A hint for two keys that do the same thing; a click presses the first.
fn hint2(key: Key, other: Key, desc: &'static str) -> Hint {
    Hint { keys: labels(&[key, other]), desc, key }
}

/// The hints the footer shows for the focus.
pub fn footer_hints(state: &State) -> Vec<Hint> {
    let (enter, esc) = (Key::Code(KeyCode::Enter), Key::Code(KeyCode::Esc));
    let (j, k, tab) = (Key::Char('j'), Key::Char('k'), Key::Code(KeyCode::Tab));
    match &state.focus {
        Focus::Filter => {
            let mut hints = vec![hint(enter, "keep"), hint(tab, "complete"), hint(esc, "clear")];
            if !state.config.saved_filters.is_empty() {
                hints.push(hint2(Key::Code(KeyCode::Down), Key::Code(KeyCode::Up), "pick a saved filter"));
            }
            hints
        }
        Focus::Message { .. } => vec![hint(enter, "send"), hint(esc, "cancel")],
        Focus::Dialog(_) => vec![hint(Key::Ctrl('s'), "start"), hint(tab, "go to next field"), hint(esc, "cancel")],
        Focus::Batch(_) => {
            vec![hint(Key::Ctrl('s'), "start"), hint(Key::Char(' '), "change mode"), hint(esc, "cancel")]
        }
        Focus::Settings(menu) if menu.edit.is_some() => vec![hint(enter, "save"), hint(esc, "cancel")],
        Focus::Settings(_) => vec![hint2(j, k, "move"), hint(enter, "change"), hint(esc, "close")],
        Focus::Keybinds { .. } | Focus::Summary { .. } | Focus::Diff { .. } => {
            vec![hint2(j, k, "scroll"), hint(esc, "close")]
        }
        Focus::List => key_bar(state),
    }
}

pub fn prompt_hint(state: &State) -> &'static str {
    match state.tab {
        Tab::Issues if can_start(state) => "/ to filter issues · s to start a session",
        Tab::Issues => "/ to filter issues",
        Tab::Prs if can_start(state) => "/ to filter pull requests · s to start a session",
        Tab::Prs => "/ to filter pull requests",
        Tab::Sessions => match current_session(state).map(|s| s.state) {
            Some(SessionState::Running | SessionState::NeedsYou) => "/ to filter sessions · a to attach",
            Some(SessionState::Exited) => "/ to filter sessions · a to resume",
            None => "/ to filter sessions",
        },
    }
}

/// The key bar for the list. A click on an entry presses its binding's first key.
pub(super) fn key_bar(state: &State) -> Vec<Hint> {
    let has_row = selected_row(state).is_some();
    let current = current_session(state);
    let session = current.map(|s| s.state);
    let has_diff = current.and_then(|s| state.work.get(&s.key)).is_some_and(|w| w.has_changes());
    let shown = |when: When| match when {
        When::Always => true,
        When::CanStart => can_start(state),
        When::Markable => markable(state),
        When::PrClosesIssue => closed_issue_of_pr(state).is_some(),
        When::SessionLive => session.is_some_and(SessionState::is_live),
        When::SessionExited => session == Some(SessionState::Exited),
        When::HasTranscriptSummary => has_transcript_summary(state),
        When::HasSession => current.is_some(),
        When::HasDiff => has_diff,
        When::Removable => removable(state),
        When::HasMergedSessions => state.tab == Tab::Sessions && has_merged_sessions(state),
        When::Mergeable => merge_target(state).is_ok(),
        When::OpenWith(with) => has_row && state.context.open_with == with,
    };
    BINDINGS
        .iter()
        .filter_map(|b| Some((b, *b.keys.first()?)))
        .flat_map(|(b, key)| b.bar.iter().map(move |e| (e, key)))
        .filter(|(e, _)| shown(e.when))
        .map(|(e, key)| hint(key, e.desc))
        .collect()
}

pub(super) fn key_of(ev: &KeyEvent) -> Option<Key> {
    let plain = ev.modifiers.difference(KeyModifiers::SHIFT).is_empty();
    match ev.code {
        KeyCode::Char(c) if ev.modifiers.contains(KeyModifiers::CONTROL) => Some(Key::Ctrl(c.to_ascii_lowercase())),
        KeyCode::Char(c) if plain => Some(Key::Char(c)),
        KeyCode::Char(_) => None,
        code => Some(Key::Code(code)),
    }
}

pub(super) fn action_for(ev: &KeyEvent) -> Option<Action> {
    let key = key_of(ev)?;
    BINDINGS.iter().find(|b| b.keys.contains(&key)).map(|b| b.action)
}

/// `armed` is what a previous `m` or `D` armed; only the next press of that key uses it.
pub(super) fn act(state: &mut State, action: Action, armed: Option<&Armed>) -> Vec<Cmd> {
    match action {
        Action::GoTab(tab) => state.tab = tab,
        Action::Down => move_selection(state, Move::Down),
        Action::Up => move_selection(state, Move::Up),
        Action::Start => return start_key(state),
        Action::Mark => mark_key(state),
        Action::Enter => match state.tab {
            Tab::Issues => return start_key(state),
            Tab::Prs => go_to_issue_of_pr(state),
            Tab::Sessions => return attach_key(state),
        },
        Action::Attach => return attach_key(state),
        Action::Stop => return stop_key(state),
        Action::Remove => return remove_key(state, armed),
        Action::Clean => return clean_key(state, armed),
        Action::Open => return open_key(state),
        Action::Merge => return merge_key(state, armed),
        Action::Refresh => {
            let mut cmds = Vec::new();
            refresh_github(state, &mut cmds);
            cmds.extend(observe_now(state));
            return cmds;
        }
        Action::Filter => state.focus = Focus::Filter,
        Action::ClearFilter => state.filter_mut().clear(),
        Action::Sort => {
            // Pin the shown row first, so the first-row fallback can not move it.
            if let Some(key) = selected_row(state) {
                select(state, key);
            }
            state.tabs.cycle_sort(state.tab);
        }
        Action::Settings => state.focus = Focus::Settings(Box::new(SettingsMenu { at: SETTINGS[0], edit: None })),
        Action::Keybinds => state.focus = Focus::Keybinds { scroll: 0 },
        Action::Summary => return summary_key(state),
        Action::Diff => return diff_key(state),
        Action::Shell => return shell_key(state),
        Action::Message => return open_message(state),
        Action::Quit => return vec![Cmd::Quit],
    }
    Vec::new()
}
