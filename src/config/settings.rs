use super::{AgentCommand, Config, ConfigError, env_path, parse, read};
use serde_json::{Map, Value, json};
use std::path::Path;

/// A setting the menu shows. One table: the menu order is `SETTINGS`, and
/// everything that varies by setting is a method here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    DetectWaiting,
    Icons,
    Multiplexer,
    Harness,
    Command,
    RemoveOnMerge,
}

pub const SETTINGS: [Setting; 6] = [
    Setting::DetectWaiting,
    Setting::Icons,
    Setting::Multiplexer,
    Setting::Harness,
    Setting::Command,
    Setting::RemoveOnMerge,
];

/// What `enter` does to a setting.
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    /// The value changed in place.
    Applied,
    /// The value is typed: edit this text, then `Setting::set_text`.
    Edit(String),
}

impl Setting {
    /// The JSON key, and the name the menu shows.
    pub fn key(self) -> &'static str {
        match self {
            Setting::DetectWaiting => "detect_waiting",
            Setting::Icons => "icons",
            Setting::Multiplexer => "multiplexer",
            Setting::Harness => "harness",
            Setting::Command => "command",
            Setting::RemoveOnMerge => "remove_on_merge",
        }
    }

    pub fn applies_note(self) -> Option<&'static str> {
        match self {
            Setting::DetectWaiting | Setting::Harness | Setting::Command => {
                Some("Applies to sessions you start or resume after this change.")
            }
            Setting::Multiplexer => Some("Applies the next time you start dispatch."),
            Setting::Icons | Setting::RemoveOnMerge => None,
        }
    }

    pub fn show(self, config: &Config) -> String {
        match self {
            Setting::DetectWaiting => (if config.detect_waiting { "on" } else { "off" }).to_string(),
            Setting::Icons => config.icons.label().to_string(),
            Setting::Multiplexer => config.multiplexer.label().to_string(),
            Setting::Harness => config.harness.label().to_string(),
            Setting::Command => config.agent_command(config.harness).words(),
            Setting::RemoveOnMerge => config.remove_on_merge.label().to_string(),
        }
    }

    /// `enter` in the menu.
    pub fn change(self, config: &mut Config) -> Change {
        match self {
            Setting::DetectWaiting => config.detect_waiting = !config.detect_waiting,
            Setting::Icons => config.icons = config.icons.next(),
            Setting::Multiplexer => config.multiplexer = config.multiplexer.next(),
            Setting::Harness => {
                config.harness = config.harness.next();
                config.command = None;
            }
            Setting::Command => return Change::Edit(Setting::Command.show(config)),
            Setting::RemoveOnMerge => config.remove_on_merge = config.remove_on_merge.toggle(),
        }
        Change::Applied
    }

    /// Typed text for a typed setting, checked like the file. `config` is
    /// unchanged on an error. A setting that is not typed ignores the text.
    pub fn set_text(self, config: &mut Config, text: &str) -> Result<(), ConfigError> {
        match self {
            Setting::Command => {
                let command = AgentCommand::from_words(text)?;
                config.command = (command != AgentCommand::default_for(config.harness)).then_some(command);
            }
            Setting::DetectWaiting
            | Setting::Icons
            | Setting::Multiplexer
            | Setting::Harness
            | Setting::RemoveOnMerge => {}
        }
        Ok(())
    }

    /// The file fields this setting writes. Cycling the harness clears the
    /// command override, so it writes both.
    fn fields(self, config: &Config) -> Vec<(&'static str, Value)> {
        let command = || {
            let argv = config.command.as_ref().map(|c| c.argv(&[]));
            ("command", argv.map_or(Value::Null, |argv| json!(argv)))
        };
        match self {
            Setting::DetectWaiting => vec![("detect_waiting", json!(config.detect_waiting))],
            Setting::Icons => vec![("icons", json!(config.icons.label()))],
            Setting::Multiplexer => vec![("multiplexer", json!(config.multiplexer.label()))],
            Setting::Harness => vec![("harness", json!(config.harness.label())), command()],
            Setting::Command => vec![command()],
            Setting::RemoveOnMerge => vec![("remove_on_merge", json!(config.remove_on_merge.as_bool()))],
        }
    }
}

/// Pure: the new file text after `setting` takes its value from `config`, or
/// `None` when there is nothing to write. `text` is the file now, `None` for
/// no file. A file that does not parse is an error, so dispatch never
/// overwrites a file the user broke. Fields the user did not change stay,
/// in their order. A value equal to the default removes its field.
fn edit_file(
    text: Option<&str>,
    path: &Path,
    setting: Setting,
    config: &Config,
) -> Result<Option<String>, ConfigError> {
    let json_error = |message: String| ConfigError::Json { path: path.to_path_buf(), message };
    let mut object = match text {
        Some(text) => {
            parse(text, path)?;
            serde_json::from_str::<Map<String, Value>>(text).map_err(|e| json_error(e.to_string()))?
        }
        None => Map::new(),
    };
    let fields = setting.fields(config);
    let defaults = setting.fields(&Config::default());
    if text.is_none() && fields == defaults {
        return Ok(None);
    }
    for ((key, value), (_, default)) in fields.into_iter().zip(defaults) {
        if value == default {
            object.shift_remove(key);
        } else {
            object.insert(key.to_string(), value);
        }
    }
    if matches!(setting, Setting::Harness | Setting::Command) {
        object.shift_remove("claude_command");
    }
    let mut out = serde_json::to_string_pretty(&object).map_err(|e| json_error(e.to_string()))?;
    out.push('\n');
    Ok(Some(out))
}

/// Writes `setting`'s value from `config` into the config file. The write goes
/// to a temp file first and is renamed over the file, so a crash leaves the old
/// file whole.
pub fn save(setting: Setting, config: &Config) -> Result<(), ConfigError> {
    let path = env_path().ok_or(ConfigError::NoConfigDir)?;
    let Some(out) = edit_file(read(&path)?.as_deref(), &path, setting, config)? else {
        return Ok(());
    };
    let write = |source| ConfigError::Write { path: path.clone(), source };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(write)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, out).map_err(write)?;
    std::fs::rename(&tmp, &path).map_err(write)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Harness, Multiplexer, RemoveOnMerge};
    use crate::glyphs::IconSet;

    fn parse_str(text: &str) -> Result<Config, ConfigError> {
        parse(text, Path::new("config.json"))
    }

    fn edit(text: Option<&str>, setting: Setting, config: &Config) -> Result<Option<String>, ConfigError> {
        edit_file(text, Path::new("config.json"), setting, config)
    }

    #[test]
    fn a_changed_field_keeps_the_others_and_their_order() {
        let before = "{\n  \"split\": 30,\n  \"icons\": \"nerdfont\",\n  \"detect_waiting\": false\n}\n";
        let config = Config { detect_waiting: true, ..parse_str(before).expect("valid") };
        let config = Config { command: Some(AgentCommand::from_words("my-claude --x").expect("valid")), ..config };
        let after = edit(Some(before), Setting::Command, &config).expect("ok").expect("written");
        let keys: Vec<_> = serde_json::from_str::<Map<String, Value>>(&after).expect("json").keys().cloned().collect();
        assert_eq!(keys, ["split", "icons", "detect_waiting", "command"]);
        assert!(after.ends_with("}\n"));
        let reread = parse_str(&after).expect("valid");
        assert_eq!(reread.icons, IconSet::NerdFont);
        assert!(!reread.detect_waiting, "only the changed field is written");
        assert_eq!(reread.agent_command(reread.harness).words(), "my-claude --x");
    }

    #[test]
    fn a_field_set_back_to_its_default_is_removed() {
        let before = r#"{"split":30,"icons":"nerdfont"}"#;
        let config = Config { icons: IconSet::Unicode, ..parse_str(before).expect("valid") };
        let after = edit(Some(before), Setting::Icons, &config).expect("ok").expect("written");
        assert!(!after.contains("icons"), "{after}");
        assert!(after.contains("split"), "{after}");
        let emptied = edit(Some(r#"{"icons":"nerdfont"}"#), Setting::Icons, &Config::default()).expect("ok");
        assert_eq!(emptied.as_deref(), Some("{}\n"));
    }

    #[test]
    fn a_changed_field_is_added_to_a_missing_or_empty_file() {
        let config = Config { detect_waiting: false, ..Config::default() };
        for text in [None, Some("{}")] {
            let after = edit(text, Setting::DetectWaiting, &config).expect("ok").expect("written");
            assert_eq!(after, "{\n  \"detect_waiting\": false\n}\n");
        }
    }

    #[test]
    fn a_default_with_no_file_writes_nothing() {
        for setting in SETTINGS {
            assert_eq!(edit(None, setting, &Config::default()).expect("ok"), None, "{setting:?}");
        }
    }

    #[test]
    fn a_broken_file_is_an_error_and_never_overwritten() {
        let config = Config { detect_waiting: false, ..Config::default() };
        for text in ["not json", r#"{"command":[]}"#, r#"{"skills":[]}"#] {
            assert!(edit(Some(text), Setting::DetectWaiting, &config).is_err(), "{text}");
        }
    }

    #[test]
    fn every_setting_round_trips_through_the_file() {
        let config = Config {
            detect_waiting: false,
            icons: IconSet::NerdFont,
            multiplexer: Multiplexer::Herdr,
            harness: Harness::Codex,
            command: Some(AgentCommand::from_words("sh -c 'echo hi'").expect("valid")),
            remove_on_merge: RemoveOnMerge::On,
            ..Config::default()
        };
        let mut text: Option<String> = None;
        for setting in SETTINGS {
            text = edit(text.as_deref(), setting, &config).expect("ok");
        }
        assert_eq!(parse_str(&text.expect("written")).expect("valid"), config);
    }

    #[test]
    fn typed_settings_check_text_like_the_file() {
        let mut config = Config::default();
        assert!(matches!(Setting::Command.set_text(&mut config, "claude 'oops"), Err(ConfigError::BadCommand(_))));
        assert_eq!(config, Config::default(), "an error changes nothing");
        Setting::Command.set_text(&mut config, "my-claude --x").expect("valid");
        assert_eq!(Setting::Command.show(&config), "my-claude --x");
        Setting::Command.set_text(&mut config, "claude").expect("valid");
        assert_eq!(config.command, None, "the default command is no override");
    }

    #[test]
    fn cycling_the_harness_clears_the_command_override() {
        let mut config = Config::default();
        Setting::Command.set_text(&mut config, "my-claude --x").expect("valid");
        assert_eq!(Setting::Harness.change(&mut config), Change::Applied);
        assert_eq!(config.harness, Harness::Codex);
        assert_eq!(config.command, None);
        assert_eq!(Setting::Command.show(&config), "codex");
    }

    #[test]
    fn saving_the_harness_also_drops_the_cleared_command_and_the_old_key() {
        let before = r#"{"claude_command":["my-claude"],"icons":"nerdfont"}"#;
        let mut config = parse_str(before).expect("valid");
        Setting::Harness.change(&mut config);
        let after = edit(Some(before), Setting::Harness, &config).expect("ok").expect("written");
        let object: Map<String, Value> = serde_json::from_str(&after).expect("json");
        assert_eq!(object.get("harness"), Some(&json!("codex")));
        assert!(!object.contains_key("command") && !object.contains_key("claude_command"), "{after}");
        assert_eq!(parse_str(&after).expect("valid"), config);
    }

    #[test]
    fn saving_the_command_writes_command_and_drops_claude_command() {
        let before = r#"{"claude_command":["my-claude"]}"#;
        let mut config = parse_str(before).expect("valid");
        Setting::Command.set_text(&mut config, "other-claude --y").expect("valid");
        let after = edit(Some(before), Setting::Command, &config).expect("ok").expect("written");
        let object: Map<String, Value> = serde_json::from_str(&after).expect("json");
        assert_eq!(object.get("command"), Some(&json!(["other-claude", "--y"])));
        assert!(!object.contains_key("claude_command"), "{after}");
        assert_eq!(parse_str(&after).expect("valid"), config);
    }

    #[test]
    fn a_command_set_back_to_the_harness_default_leaves_the_file_without_it() {
        let before = r#"{"harness":"codex","command":["my-codex"]}"#;
        let mut config = parse_str(before).expect("valid");
        Setting::Command.set_text(&mut config, "codex").expect("valid");
        let after = edit(Some(before), Setting::Command, &config).expect("ok").expect("written");
        assert_eq!(after, "{\n  \"harness\": \"codex\"\n}\n");
    }
}
