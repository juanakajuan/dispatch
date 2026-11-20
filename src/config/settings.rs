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
    ClaudeCommand,
    RemoveOnMerge,
}

pub const SETTINGS: [Setting; 5] =
    [Setting::DetectWaiting, Setting::Icons, Setting::Multiplexer, Setting::ClaudeCommand, Setting::RemoveOnMerge];

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
            Setting::ClaudeCommand => "claude_command",
            Setting::RemoveOnMerge => "remove_on_merge",
        }
    }

    pub fn applies_note(self) -> Option<&'static str> {
        match self {
            Setting::DetectWaiting | Setting::ClaudeCommand => {
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
            Setting::ClaudeCommand => config.claude_command.words(),
            Setting::RemoveOnMerge => config.remove_on_merge.label().to_string(),
        }
    }

    /// `enter` in the menu.
    pub fn change(self, config: &mut Config) -> Change {
        match self {
            Setting::DetectWaiting => config.detect_waiting = !config.detect_waiting,
            Setting::Icons => config.icons = config.icons.next(),
            Setting::Multiplexer => config.multiplexer = config.multiplexer.next(),
            Setting::ClaudeCommand => return Change::Edit(config.claude_command.words()),
            Setting::RemoveOnMerge => config.remove_on_merge = config.remove_on_merge.toggle(),
        }
        Change::Applied
    }

    /// Typed text for a typed setting, checked like the file. `config` is
    /// unchanged on an error. A setting that is not typed ignores the text.
    pub fn set_text(self, config: &mut Config, text: &str) -> Result<(), ConfigError> {
        match self {
            Setting::ClaudeCommand => config.claude_command = AgentCommand::from_words(text)?,
            Setting::DetectWaiting | Setting::Icons | Setting::Multiplexer | Setting::RemoveOnMerge => {}
        }
        Ok(())
    }

    fn json(self, config: &Config) -> Value {
        match self {
            Setting::DetectWaiting => json!(config.detect_waiting),
            Setting::Icons => json!(config.icons.label()),
            Setting::Multiplexer => json!(config.multiplexer.label()),
            Setting::ClaudeCommand => json!(config.claude_command.argv(&[])),
            Setting::RemoveOnMerge => json!(config.remove_on_merge.as_bool()),
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
    let value = setting.json(config);
    if value == setting.json(&Config::default()) {
        if text.is_none() {
            return Ok(None);
        }
        object.shift_remove(setting.key());
    } else {
        object.insert(setting.key().to_string(), value);
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
    use crate::config::{Multiplexer, RemoveOnMerge};
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
        let config = Config { claude_command: AgentCommand::from_words("my-claude --x").expect("valid"), ..config };
        let after = edit(Some(before), Setting::ClaudeCommand, &config).expect("ok").expect("written");
        let keys: Vec<_> = serde_json::from_str::<Map<String, Value>>(&after).expect("json").keys().cloned().collect();
        assert_eq!(keys, ["split", "icons", "detect_waiting", "claude_command"]);
        assert!(after.ends_with("}\n"));
        let reread = parse_str(&after).expect("valid");
        assert_eq!(reread.icons, IconSet::NerdFont);
        assert!(!reread.detect_waiting, "only the changed field is written");
        assert_eq!(reread.claude_command.words(), "my-claude --x");
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
        for text in ["not json", r#"{"claude_command":[]}"#, r#"{"skills":[]}"#] {
            assert!(edit(Some(text), Setting::DetectWaiting, &config).is_err(), "{text}");
        }
    }

    #[test]
    fn every_setting_round_trips_through_the_file() {
        let config = Config {
            detect_waiting: false,
            icons: IconSet::NerdFont,
            multiplexer: Multiplexer::Herdr,
            claude_command: AgentCommand::from_words("sh -c 'echo hi'").expect("valid"),
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
        assert!(matches!(
            Setting::ClaudeCommand.set_text(&mut config, "claude 'oops"),
            Err(ConfigError::BadCommand(_))
        ));
        assert_eq!(config, Config::default(), "an error changes nothing");
        Setting::ClaudeCommand.set_text(&mut config, "my-claude --x").expect("valid");
        assert_eq!(Setting::ClaudeCommand.show(&config), "my-claude --x");
    }
}
