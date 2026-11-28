use super::*;
use crate::config::Multiplexer;

#[test]
fn question_mark_opens_and_closes_the_keybinds_menu() {
    let (mut state, _) = app();
    assert_eq!(
        key_bar(&state).first().map(|b| (b.keys.as_str(), b.desc)),
        Some(("?", "show keys")),
        "a narrow terminal still shows it"
    );
    assert!(press(&mut state, key('?')).is_empty());
    assert!(matches!(state.focus, Focus::Keybinds { scroll: 0 }));
    assert!(press(&mut state, key('q')).is_empty(), "list keys do not act while the menu is open");
    assert!(press(&mut state, key('j')).is_empty());
    assert!(matches!(state.focus, Focus::Keybinds { scroll: 0 }), "a menu that fits does not scroll");
    press(&mut state, key('?'));
    assert!(matches!(state.focus, Focus::List), "? closes it again");
    press(&mut state, key('?'));
    press(&mut state, code(KeyCode::Esc));
    assert!(matches!(state.focus, Focus::List), "esc closes it");
}

fn menu(state: &State) -> &SettingsMenu {
    match &state.focus {
        Focus::Settings(m) => m,
        _ => panic!("the settings menu is not open"),
    }
}

fn open_menu_at(state: &mut State, setting: Setting) {
    press(state, key(','));
    move_to(state, setting);
}

/// Moves to `setting` in the menu that is open, down or up.
fn move_to(state: &mut State, setting: Setting) {
    let index = |s| SETTINGS.iter().position(|x| *x == s).expect("a setting");
    while menu(state).at != setting {
        press(state, key(if index(menu(state).at) < index(setting) { 'j' } else { 'k' }));
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

fn saves(cmds: &[Cmd]) -> Vec<(&[Setting], &Config)> {
    cmds.iter()
        .filter_map(|c| match c {
            Cmd::Job(Job::Write(WriteJob::SaveConfig { settings, config })) => Some((settings.as_slice(), config)),
            _ => None,
        })
        .collect()
}

#[test]
fn comma_opens_and_closes_the_settings_menu() {
    let (mut state, _) = app();
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

fn pending(state: &State) -> &Config {
    &menu(state).pending
}

#[test]
fn arrows_change_the_pending_value_and_do_not_save() {
    let (mut state, _) = app();
    press(&mut state, key(','));
    for k in [code(KeyCode::Right), key('l'), code(KeyCode::Left), key('h')] {
        let before = pending(&state).detect_waiting;
        assert!(press(&mut state, k).is_empty(), "no save");
        assert_ne!(pending(&state).detect_waiting, before, "each key toggles an on/off setting");
    }
    press(&mut state, code(KeyCode::Right));
    assert!(!pending(&state).detect_waiting);
    assert!(state.config.detect_waiting, "the live config waits for enter");
}

#[test]
fn left_and_right_go_back_and_forth_through_the_values() {
    let (mut state, _) = app();
    open_menu_at(&mut state, Setting::Multiplexer);
    press(&mut state, code(KeyCode::Left));
    assert_eq!(pending(&state).multiplexer, Multiplexer::HerdrApp);
    press(&mut state, code(KeyCode::Right));
    press(&mut state, code(KeyCode::Right));
    assert_eq!(pending(&state).multiplexer, Multiplexer::Tmux);
    press(&mut state, key('h'));
    assert_eq!(pending(&state).multiplexer, Multiplexer::Auto);
}

#[test]
fn enter_saves_every_change_in_one_write_and_closes() {
    let (mut state, _) = app();
    press(&mut state, key(','));
    press(&mut state, code(KeyCode::Right));
    press(&mut state, key('j'));
    press(&mut state, code(KeyCode::Right));
    move_to(&mut state, Setting::RemoveOnMerge);
    press(&mut state, key('l'));
    let cmds = press(&mut state, code(KeyCode::Enter));
    assert!(matches!(state.focus, Focus::List), "enter closes the menu");
    let [Cmd::Job(Job::Write(WriteJob::SaveConfig { settings, config }))] = cmds.as_slice() else {
        panic!("one save: {}", cmds.len())
    };
    assert_eq!(settings, &[Setting::DetectWaiting, Setting::Icons, Setting::RemoveOnMerge]);
    assert_eq!(config, &state.config);
    assert!(!state.config.detect_waiting);
    assert_eq!(state.config.icons, IconSet::NerdFont);
    assert_eq!(state.config.remove_on_merge, crate::config::RemoveOnMerge::On);
}

#[test]
fn enter_with_no_change_closes_and_writes_nothing() {
    let (mut state, _) = app();
    press(&mut state, key(','));
    press(&mut state, code(KeyCode::Right));
    press(&mut state, code(KeyCode::Left));
    assert!(press(&mut state, code(KeyCode::Enter)).is_empty());
    assert!(matches!(state.focus, Focus::List));
    assert_eq!(state.config, Config::default());
}

#[test]
fn esc_and_comma_close_and_drop_the_changes() {
    for close in [code(KeyCode::Esc), key(',')] {
        let (mut state, _) = app();
        open_menu_at(&mut state, Setting::Icons);
        press(&mut state, code(KeyCode::Right));
        assert!(press(&mut state, close).is_empty());
        assert!(matches!(state.focus, Focus::List));
        assert_eq!(state.config, Config::default());
        press(&mut state, key(','));
        assert_eq!(*pending(&state), Config::default(), "a new menu starts from the saved config");
    }
}

#[test]
fn a_pending_harness_shows_the_command_it_gives() {
    let (mut state, _) = app();
    open_menu_at(&mut state, Setting::Command);
    press(&mut state, code(KeyCode::Enter));
    type_text(&mut state, " --x");
    press(&mut state, code(KeyCode::Enter));
    press(&mut state, key(','));
    move_to(&mut state, Setting::Harness);
    press(&mut state, code(KeyCode::Right));
    assert_eq!(Setting::Command.show(pending(&state)), "codex");
    assert_eq!(state.config.agent_command(state.config.harness).words(), "claude --x", "not saved yet");
    let cmds = press(&mut state, code(KeyCode::Enter));
    assert_eq!(saves(&cmds).len(), 1);
    assert_eq!(state.config.command, None);
}

#[test]
fn a_harness_changed_and_changed_back_keeps_the_command() {
    let (mut state, _) = app();
    open_menu_at(&mut state, Setting::Command);
    press(&mut state, code(KeyCode::Enter));
    type_text(&mut state, " --x");
    press(&mut state, code(KeyCode::Enter));
    let saved = state.config.clone();
    press(&mut state, key(','));
    move_to(&mut state, Setting::Harness);
    press(&mut state, code(KeyCode::Right));
    press(&mut state, code(KeyCode::Left));
    assert_eq!(*pending(&state), saved);
    assert!(press(&mut state, code(KeyCode::Enter)).is_empty(), "nothing changed, so nothing is written");
    assert_eq!(state.config, saved);
}

#[test]
fn arrows_do_not_change_a_typed_setting() {
    let (mut state, _) = app();
    open_menu_at(&mut state, Setting::Command);
    press(&mut state, code(KeyCode::Right));
    press(&mut state, code(KeyCode::Left));
    assert_eq!(*pending(&state), Config::default());
    assert!(menu(&state).edit.is_none());
}

#[test]
fn an_invalid_command_shows_an_error_and_keeps_editing() {
    for bad in ["claude 'oops", "''", ""] {
        let (mut state, _) = app();
        open_menu_at(&mut state, Setting::Command);
        press(&mut state, code(KeyCode::Enter));
        clear_input(&mut state);
        type_text(&mut state, bad);
        let cmds = press(&mut state, code(KeyCode::Enter));
        assert!(cmds.is_empty(), "{bad:?}");
        let edit = menu(&state).edit.as_ref().expect("still editing");
        assert!(edit.error.is_some(), "{bad:?}");
        assert_eq!(state.config, Config::default());
        type_text(&mut state, "x");
        assert!(menu(&state).edit.as_ref().is_some_and(|e| e.error.is_none()), "typing clears the error");
    }
}

#[test]
fn the_command_error_says_what_the_file_says() {
    let (mut state, _) = app();
    open_menu_at(&mut state, Setting::Command);
    press(&mut state, code(KeyCode::Enter));
    type_text(&mut state, " 'oops");
    press(&mut state, code(KeyCode::Enter));
    let error = menu(&state).edit.as_ref().and_then(|e| e.error.clone());
    assert_eq!(error, Some("invalid config: command has a quote that never closes".to_string()));
}

#[test]
fn a_command_edit_applies_and_saves() {
    let (mut state, _) = app();
    open_menu_at(&mut state, Setting::Command);
    press(&mut state, code(KeyCode::Enter));
    type_text(&mut state, " --model 'big one'");
    let cmds = press(&mut state, code(KeyCode::Enter));
    assert_eq!(state.config.agent_command(state.config.harness).argv(&[]), ["claude", "--model", "big one"]);
    assert_eq!(saves(&cmds)[0].0, [Setting::Command]);
    assert!(matches!(state.focus, Focus::List), "enter in the editor saves and closes the menu");
}

#[test]
fn a_blank_command_is_refused() {
    let (mut state, _) = app();
    open_menu_at(&mut state, Setting::Command);
    press(&mut state, code(KeyCode::Enter));
    clear_input(&mut state);
    assert!(press(&mut state, code(KeyCode::Enter)).is_empty());
    assert!(menu(&state).edit.as_ref().is_some_and(|e| e.error.is_some()));
    assert_eq!(state.config, Config::default());
}

#[test]
fn esc_while_editing_cancels_the_edit_only() {
    let (mut state, _) = app();
    open_menu_at(&mut state, Setting::Command);
    press(&mut state, code(KeyCode::Enter));
    type_text(&mut state, " --x");
    press(&mut state, code(KeyCode::Esc));
    assert!(menu(&state).edit.is_none(), "the menu stays open");
    assert_eq!(menu(&state).at, Setting::Command);
    assert_eq!(state.config, Config::default());
    press(&mut state, code(KeyCode::Esc));
    assert!(matches!(state.focus, Focus::List));
}

#[test]
fn keys_in_the_editor_do_not_move_or_close_the_menu() {
    let (mut state, _) = app();
    open_menu_at(&mut state, Setting::Command);
    press(&mut state, code(KeyCode::Enter));
    type_text(&mut state, "j,q");
    assert_eq!(menu(&state).at, Setting::Command);
    assert_eq!(menu(&state).edit.as_ref().map(|e| e.input.text()), Some("claudej,q"));
}

#[test]
fn a_new_start_uses_the_changed_settings() {
    let (mut state, _) = app();
    press(&mut state, key(','));
    press(&mut state, code(KeyCode::Right));
    press(&mut state, code(KeyCode::Enter));
    press(&mut state, key('s'));
    let cmds = press(&mut state, ctrl('s'));
    let [Cmd::Job(Job::Write(WriteJob::Start(_, launch)))] = cmds.as_slice() else { panic!("one start") };
    assert!(!launch.detect_waiting, "the toggle reaches the next start");
    assert_eq!(*launch, Launch::from_config(&state.config));
}

#[test]
fn a_resume_uses_the_changed_command() {
    let (mut state, _) = app();
    state.sessions.insert(issue_key(3), session(3, SessionState::Exited));
    state
        .transcripts
        .insert(issue_key(3), crate::transcript::Transcript { said: None, active_at: jiff::Timestamp::UNIX_EPOCH });
    open_menu_at(&mut state, Setting::Command);
    press(&mut state, code(KeyCode::Enter));
    type_text(&mut state, " --x");
    press(&mut state, code(KeyCode::Enter));
    press(&mut state, key('3'));
    let cmds = press(&mut state, key('a'));
    let [Cmd::Job(Job::Write(WriteJob::Resume(_, launch)))] = cmds.as_slice() else { panic!("one resume") };
    assert_eq!(launch.command.words(), "claude --x");
}

#[test]
fn a_failed_save_toasts_and_a_good_one_does_not() {
    let (mut state, _) = app();
    assert!(press(&mut state, Msg::Saved(Ok(()))).is_empty());
    assert!(state.toast.is_none());
    let source = std::io::Error::other("disk full");
    press(&mut state, Msg::Saved(Err(crate::config::ConfigError::Write { path: "/c/dispatch.json".into(), source })));
    assert_eq!(
        state.toast.as_ref().map(Toast::text),
        Some("Saving settings failed: cannot write /c/dispatch.json: disk full.")
    );
}

fn menu_with_rows() -> (State, Instant) {
    let (mut state, now) = app();
    press(&mut state, key(','));
    state.hits.targets = vec![
        (Rect::new(0, 0, 20, 1), HitTarget::Setting(Setting::DetectWaiting)),
        (Rect::new(0, 1, 20, 1), HitTarget::Setting(Setting::Icons)),
        (Rect::new(0, 3, 20, 1), HitTarget::Setting(Setting::Command)),
        (Rect::new(30, 0, 5, 1), HitTarget::Key(Key::Code(KeyCode::Esc))),
    ];
    (state, now)
}

#[test]
fn a_click_selects_a_settings_row_and_a_second_click_changes_it_without_saving() {
    let (mut state, now) = menu_with_rows();
    assert!(press(&mut state, click(1, 1, now)).is_empty(), "the first click only selects");
    assert_eq!(menu(&state).at, Setting::Icons);
    let cmds = press(&mut state, click(1, 1, now + Duration::from_millis(100)));
    assert!(cmds.is_empty(), "a click never saves");
    assert_eq!(pending(&state).icons, IconSet::NerdFont, "the second click selects the next value");
    assert_eq!(state.config, Config::default());
}

#[test]
fn clicking_a_typed_setting_twice_opens_its_editor_and_rows_stop_reacting_there() {
    let (mut state, now) = menu_with_rows();
    press(&mut state, click(1, 3, now));
    press(&mut state, click(1, 3, now + Duration::from_secs(1)));
    assert!(menu(&state).edit.is_some());
    press(&mut state, click(1, 0, now + Duration::from_secs(2)));
    press(&mut state, click(1, 0, now + Duration::from_secs(3)));
    assert_eq!((menu(&state).at, pending(&state)), (Setting::Command, &Config::default()));
}

#[test]
fn the_close_control_closes_the_settings_menu() {
    let (mut state, now) = menu_with_rows();
    press(&mut state, click(31, 0, now));
    assert!(matches!(state.focus, Focus::List));
}

#[test]
fn the_wheel_moves_through_the_settings_and_scrolls_the_keybinds() {
    let (mut state, now) = app();
    press(&mut state, key(','));
    press(&mut state, wheel(MouseEventKind::ScrollDown, now));
    press(&mut state, wheel(MouseEventKind::ScrollDown, now));
    assert_eq!(menu(&state).at, SETTINGS[2]);
    press(&mut state, wheel(MouseEventKind::ScrollUp, now));
    assert_eq!(menu(&state).at, SETTINGS[1]);
    press(&mut state, code(KeyCode::Esc));

    press(&mut state, key('?'));
    state.hits.max_scroll = 5;
    press(&mut state, wheel(MouseEventKind::ScrollDown, now));
    press(&mut state, wheel(MouseEventKind::ScrollDown, now));
    assert!(matches!(state.focus, Focus::Keybinds { scroll: 2 }));
    press(&mut state, wheel(MouseEventKind::ScrollUp, now));
    assert!(matches!(state.focus, Focus::Keybinds { scroll: 1 }));
}
