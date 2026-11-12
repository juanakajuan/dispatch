//! Optional user config at `$XDG_CONFIG_HOME/dispatch/config.json`, falling
//! back to `~/.config/dispatch/config.json`. A missing file gives
//! `Config::default()`. A present but invalid file is a startup error, not a
//! silent fallback. Every field is optional, and a present field replaces the
//! default one.
//!
//! The settings menu writes a change back with `save`. It keeps every field
//! the user did not change, and removes a field set back to its default, so
//! the file never holds defaults dispatch put there.
//!
//! ```json
//! {
//!   "detect_waiting": true,
//!   "claude_command": ["claude"],
//!   "split": 42,
//!   "icons": "unicode"
//! }
//! ```

use crate::domain::Split;
use crate::glyphs::IconSet;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The program and leading args that start Claude. Dispatch appends its own
/// args after these. The program is separate from the args, so an empty
/// command can not exist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl ClaudeCommand {
    /// The full argv for a start or resume.
    pub fn argv(&self, tail: &[String]) -> Vec<String> {
        let mut argv = vec![self.program.clone()];
        argv.extend(self.args.iter().cloned());
        argv.extend(tail.iter().cloned());
        argv
    }

    /// The config boundary: an argv with a program and no blank program name.
    pub fn from_argv(argv: Vec<String>) -> Result<ClaudeCommand, ConfigError> {
        let mut parts = argv.into_iter();
        let program = parts.next().ok_or(ConfigError::BadCommand("is empty"))?;
        if program.trim().is_empty() {
            return Err(ConfigError::BadCommand("starts with an empty program name"));
        }
        Ok(ClaudeCommand { program, args: parts.collect() })
    }

    /// One line of words, for editing. Split on whitespace. `'...'` and
    /// `"..."` group words, and there are no escapes.
    pub fn from_words(text: &str) -> Result<ClaudeCommand, ConfigError> {
        let mut argv = Vec::new();
        let mut word = String::new();
        let mut started = false;
        let mut quote: Option<char> = None;
        for c in text.chars() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => word.push(c),
                (None, '\'' | '"') => {
                    quote = Some(c);
                    started = true;
                }
                (None, c) if c.is_whitespace() => {
                    if started {
                        argv.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                (None, c) => {
                    word.push(c);
                    started = true;
                }
            }
        }
        if quote.is_some() {
            return Err(ConfigError::BadCommand("has a quote that never closes"));
        }
        if started {
            argv.push(word);
        }
        ClaudeCommand::from_argv(argv)
    }

    /// The inverse of `from_words`.
    pub fn words(&self) -> String {
        let quoted = |arg: &str| {
            let plain = !arg.is_empty() && !arg.contains(|c: char| c.is_whitespace() || c == '\'' || c == '"');
            match (plain, arg.contains('\''), arg.contains('"')) {
                (true, _, _) => arg.to_string(),
                (false, false, _) => format!("'{arg}'"),
                (false, true, false) => format!("\"{arg}\""),
                // Both quotes: single-quote each run, and put each `'` between them in double quotes.
                (false, true, true) => arg.split('\'').map(|run| format!("'{run}'")).collect::<Vec<_>>().join("\"'\""),
            }
        };
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(quoted)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub github_refresh: Duration,
    /// On by default. Sessions get Claude hooks that flag "waiting for you"
    /// on their tmux window (see `session::WAITING_HOOKS`). Turn it off if the
    /// `--settings` hooks interfere with the user's own hooks.
    pub detect_waiting: bool,
    pub claude_command: ClaudeCommand,
    /// Where the list | detail border starts, and where `=` puts it back.
    /// `<`, `>` and drags stay in memory. The settings menu saves this field.
    pub split: Split,
    /// Which glyph set marks states. `NerdFont` needs a Nerd Font in the terminal.
    pub icons: IconSet,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            github_refresh: Duration::from_mins(1),
            detect_waiting: true,
            claude_command: ClaudeCommand { program: "claude".to_string(), args: Vec::new() },
            split: Split::DEFAULT,
            icons: IconSet::default(),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Read { path: PathBuf, source: std::io::Error },
    Json { path: PathBuf, message: String },
    BadCommand(&'static str),
    BadSplit(u64),
    NotANumber(String),
    Write { path: PathBuf, source: std::io::Error },
    NoConfigDir,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            ConfigError::Json { path, message } => write!(f, "invalid config {}: {message}", path.display()),
            ConfigError::BadCommand(why) => write!(f, "invalid config: claude_command {why}"),
            ConfigError::BadSplit(value) => {
                let (lo, hi) = (Split::PERCENT.start(), Split::PERCENT.end());
                write!(f, "invalid config: split {value} is not a percent from {lo} to {hi}")
            }
            ConfigError::NotANumber(text) => write!(f, "invalid config: split {text:?} is not a number"),
            ConfigError::Write { path, source } => write!(f, "cannot write {}: {source}", path.display()),
            ConfigError::NoConfigDir => f.write_str("cannot save: neither XDG_CONFIG_HOME nor HOME is set"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    detect_waiting: Option<bool>,
    claude_command: Option<Vec<String>>,
    split: Option<u64>,
    icons: Option<IconSet>,
}

/// Pure: config text to `Config`. `path` only labels errors.
pub fn parse(text: &str, path: &Path) -> Result<Config, ConfigError> {
    let file: ConfigFile = serde_json::from_str(text)
        .map_err(|e| ConfigError::Json { path: path.to_path_buf(), message: e.to_string() })?;
    let mut config = Config::default();
    if let Some(detect) = file.detect_waiting {
        config.detect_waiting = detect;
    }
    if let Some(argv) = file.claude_command {
        config.claude_command = ClaudeCommand::from_argv(argv)?;
    }
    if let Some(percent) = file.split {
        config.split = Split::from_percent(percent).ok_or(ConfigError::BadSplit(percent))?;
    }
    if let Some(icons) = file.icons {
        config.icons = icons;
    }
    Ok(config)
}

pub fn path(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|s| !s.is_empty()) {
        Some(xdg) => PathBuf::from(xdg),
        None => PathBuf::from(home.filter(|s| !s.is_empty())?).join(".config"),
    };
    Some(base.join("dispatch").join("config.json"))
}

fn env_path() -> Option<PathBuf> {
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    path(xdg.as_deref(), home.as_deref())
}

/// The file's text, or `None` when there is no file.
fn read(path: &Path) -> Result<Option<String>, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ConfigError::Read { path: path.to_path_buf(), source }),
    }
}

pub fn load() -> Result<Config, ConfigError> {
    let Some(path) = env_path() else {
        return Ok(Config::default());
    };
    match read(&path)? {
        Some(text) => parse(&text, &path),
        None => Ok(Config::default()),
    }
}

/// A setting the menu shows. One table: the menu order is `SETTINGS`, and
/// everything that varies by setting is a method here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    DetectWaiting,
    Icons,
    Split,
    ClaudeCommand,
}

pub const SETTINGS: [Setting; 4] = [Setting::DetectWaiting, Setting::Icons, Setting::Split, Setting::ClaudeCommand];

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
            Setting::Split => "split",
            Setting::ClaudeCommand => "claude_command",
        }
    }

    /// True when a running session keeps the value it started with.
    pub fn new_sessions_only(self) -> bool {
        match self {
            Setting::DetectWaiting | Setting::ClaudeCommand => true,
            Setting::Icons | Setting::Split => false,
        }
    }

    pub fn show(self, config: &Config) -> String {
        match self {
            Setting::DetectWaiting => (if config.detect_waiting { "on" } else { "off" }).to_string(),
            Setting::Icons => config.icons.label().to_string(),
            Setting::Split => format!("{}%", config.split.percent()),
            Setting::ClaudeCommand => config.claude_command.words(),
        }
    }

    /// `enter` in the menu.
    pub fn change(self, config: &mut Config) -> Change {
        match self {
            Setting::DetectWaiting => config.detect_waiting = !config.detect_waiting,
            Setting::Icons => config.icons = config.icons.next(),
            Setting::Split => return Change::Edit(config.split.percent().to_string()),
            Setting::ClaudeCommand => return Change::Edit(config.claude_command.words()),
        }
        Change::Applied
    }

    /// Typed text for a typed setting, checked like the file. `config` is
    /// unchanged on an error. A setting that is not typed ignores the text.
    pub fn set_text(self, config: &mut Config, text: &str) -> Result<(), ConfigError> {
        match self {
            Setting::Split => {
                let text = text.trim();
                let percent = text.parse::<u64>().map_err(|_| ConfigError::NotANumber(text.to_string()))?;
                config.split = Split::from_percent(percent).ok_or(ConfigError::BadSplit(percent))?;
            }
            Setting::ClaudeCommand => config.claude_command = ClaudeCommand::from_words(text)?,
            Setting::DetectWaiting | Setting::Icons => {}
        }
        Ok(())
    }

    fn json(self, config: &Config) -> Value {
        match self {
            Setting::DetectWaiting => json!(config.detect_waiting),
            Setting::Icons => json!(config.icons.label()),
            Setting::Split => json!(config.split.percent()),
            Setting::ClaudeCommand => json!(config.claude_command.argv(&[])),
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

    fn parse_str(text: &str) -> Result<Config, ConfigError> {
        parse(text, Path::new("config.json"))
    }

    #[test]
    fn default_covers_every_literal() {
        let c = Config::default();
        assert!(c.detect_waiting);
        assert_eq!(c.claude_command.argv(&["x".to_string()]), ["claude", "x"]);
        assert_eq!(c.split, Split::DEFAULT);
        assert_eq!(c.icons, IconSet::Unicode);
    }

    #[test]
    fn empty_object_is_the_default() {
        let c = parse_str("{}").expect("valid");
        assert!(c.detect_waiting);
        assert_eq!(c.claude_command, Config::default().claude_command);
    }

    #[test]
    fn fields_replace_defaults() {
        let c = parse_str(
            r#"{"detect_waiting":false,"claude_command":["sh","-c","echo","_"],"split":30,"icons":"nerdfont"}"#,
        )
        .expect("valid");
        assert!(!c.detect_waiting);
        assert_eq!(c.claude_command.program, "sh");
        assert_eq!(c.claude_command.args, ["-c", "echo", "_"]);
        assert_eq!(c.split.list_width(100), 30);
        assert_eq!(c.icons, IconSet::NerdFont);
    }

    #[test]
    fn invalid_files_are_errors() {
        assert!(matches!(parse_str("not json"), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"skills":[]}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"claude_command":[]}"#), Err(ConfigError::BadCommand(_))));
        assert!(matches!(parse_str(r#"{"claude_command":[" "]}"#), Err(ConfigError::BadCommand(_))));
        assert!(matches!(parse_str(r#"{"detect_waiting":"yes"}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"split":95}"#), Err(ConfigError::BadSplit(95))));
        assert!(matches!(parse_str(r#"{"split":-1}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"split":"42"}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"icons":"emoji"}"#), Err(ConfigError::Json { .. })));
    }

    #[test]
    fn path_prefers_xdg_then_home() {
        assert_eq!(path(Some("/x"), Some("/h")), Some(PathBuf::from("/x/dispatch/config.json")));
        assert_eq!(path(Some(""), Some("/h")), Some(PathBuf::from("/h/.config/dispatch/config.json")));
        assert_eq!(path(None, None), None);
    }

    fn edit(text: Option<&str>, setting: Setting, config: &Config) -> Result<Option<String>, ConfigError> {
        edit_file(text, Path::new("config.json"), setting, config)
    }

    #[test]
    fn a_changed_field_keeps_the_others_and_their_order() {
        let before = "{\n  \"split\": 30,\n  \"icons\": \"nerdfont\",\n  \"detect_waiting\": false\n}\n";
        let config = Config { detect_waiting: true, ..parse_str(before).expect("valid") };
        let config = Config { claude_command: ClaudeCommand::from_words("my-claude --x").expect("valid"), ..config };
        let after = edit(Some(before), Setting::ClaudeCommand, &config).expect("ok").expect("written");
        let keys: Vec<_> = serde_json::from_str::<Map<String, Value>>(&after).expect("json").keys().cloned().collect();
        assert_eq!(keys, ["split", "icons", "detect_waiting", "claude_command"]);
        assert!(after.ends_with("}\n"));
        let reread = parse_str(&after).expect("valid");
        assert_eq!(reread.split.percent(), 30);
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
        let emptied = edit(Some(r#"{"split":30}"#), Setting::Split, &Config::default()).expect("ok");
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
        for text in ["not json", r#"{"split":95}"#, r#"{"skills":[]}"#] {
            assert!(edit(Some(text), Setting::DetectWaiting, &config).is_err(), "{text}");
        }
    }

    #[test]
    fn every_setting_round_trips_through_the_file() {
        let config = Config {
            detect_waiting: false,
            icons: IconSet::NerdFont,
            split: Split::from_percent(65).expect("in range"),
            claude_command: ClaudeCommand::from_words("sh -c 'echo hi'").expect("valid"),
            ..Config::default()
        };
        let mut text: Option<String> = None;
        for setting in SETTINGS {
            text = edit(text.as_deref(), setting, &config).expect("ok");
        }
        assert_eq!(parse_str(&text.expect("written")).expect("valid"), config);
    }

    #[test]
    fn words_and_from_words_round_trip() {
        for argv in [
            vec!["claude"],
            vec!["sh", "-c", "echo hi"],
            vec!["a", "", "b c"],
            vec!["x", "it's", "say \"hi\"", "both ' and \" here"],
        ] {
            let command = ClaudeCommand::from_argv(argv.iter().map(ToString::to_string).collect()).expect("valid");
            let again = ClaudeCommand::from_words(&command.words()).expect("valid");
            assert_eq!(again, command, "{}", command.words());
        }
        assert_eq!(ClaudeCommand::from_words("sh -c 'echo hi'").expect("valid").args, ["-c", "echo hi"]);
        assert_eq!(ClaudeCommand::from_words("a\"b c\"d").expect("valid").program, "ab cd");
    }

    #[test]
    fn words_reject_an_empty_or_blank_program_or_an_open_quote() {
        for text in ["", "   ", "''", "' ' x", "claude 'oops"] {
            assert!(matches!(ClaudeCommand::from_words(text), Err(ConfigError::BadCommand(_))), "{text:?}");
        }
    }

    #[test]
    fn split_percent_is_the_inverse_of_from_percent() {
        for percent in Split::PERCENT {
            assert_eq!(Split::from_percent(percent).expect("in range").percent(), percent);
        }
        assert_eq!(Split::DEFAULT.percent(), 42);
    }

    #[test]
    fn typed_settings_check_text_like_the_file() {
        let mut config = Config::default();
        assert!(matches!(Setting::Split.set_text(&mut config, "95"), Err(ConfigError::BadSplit(95))));
        assert!(matches!(Setting::Split.set_text(&mut config, "abc"), Err(ConfigError::NotANumber(_))));
        assert_eq!(config, Config::default(), "an error changes nothing");
        Setting::Split.set_text(&mut config, " 60 ").expect("valid");
        assert_eq!(Setting::Split.show(&config), "60%");
    }

    #[test]
    fn icon_labels_match_the_file_spelling() {
        for set in [IconSet::Unicode, IconSet::NerdFont] {
            let parsed: IconSet = serde_json::from_str(&format!("\"{}\"", set.label())).expect("label parses");
            assert_eq!(parsed, set);
            assert_eq!(set.next().next(), set);
        }
    }
}
