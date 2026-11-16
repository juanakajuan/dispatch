use super::feed::{observe_now, refresh_github};
use super::list::{Move, Tab, current_session, go_to_issue_of_pr, move_selection, select, selected_row};
use super::menus::SettingsMenu;
use super::merge::{Armed, merge_key, merge_target};
use super::msg::Cmd;
use super::open::open_key;
use super::sessions::{attach_key, start_key, stop_key};
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

pub(super) const fn bar(keys: &'static str, desc: &'static str, when: When) -> BarEntry {
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

/// The hints the footer shows for the focus: (keys, what they do).
pub fn footer_hints(state: &State) -> Vec<(&'static str, &'static str)> {
    match &state.focus {
        Focus::Filter => vec![("enter", "keep"), ("esc", "clear")],
        Focus::Dialog(_) => vec![("ctrl-s", "start"), ("tab", "next field"), ("esc", "cancel")],
        Focus::Settings(menu) if menu.edit.is_some() => vec![("enter", "save"), ("esc", "cancel")],
        Focus::Settings(_) => vec![("j/k", "move"), ("enter", "change"), ("esc", "close")],
        Focus::Keybinds { .. } => vec![("j/k", "scroll"), ("esc", "close")],
        Focus::List => key_bar(state),
    }
}

/// The key bar for the list: (keys, description).
pub(super) fn key_bar(state: &State) -> Vec<(&'static str, &'static str)> {
    let session = current_session(state).map(|s| s.state);
    let shown = |when: When| match when {
        When::Always => true,
        When::OnIssues => state.tab == Tab::Issues,
        When::OnPrs => state.tab == Tab::Prs,
        When::OnSessions => state.tab == Tab::Sessions && session.is_some(),
        When::SessionLive => session.is_some_and(SessionState::is_live),
        When::SessionExited => session == Some(SessionState::Exited),
        When::Mergeable => merge_target(state).is_ok(),
        When::OpenWith(with) => state.context.open_with == with,
    };
    BINDINGS.iter().flat_map(|b| b.bar).filter(|e| shown(e.when)).map(|e| (e.keys, e.desc)).collect()
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

/// `armed` is what a previous `m` armed; only the next `m` uses it.
pub(super) fn act(state: &mut State, action: Action, armed: Option<&Armed>) -> Vec<Cmd> {
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
        Action::Stop => return stop_key(state),
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
