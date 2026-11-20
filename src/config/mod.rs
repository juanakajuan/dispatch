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
//!   "icons": "unicode",
//!   "multiplexer": "auto",
//!   "remove_on_merge": false
//! }
//! ```

pub mod settings;

use crate::glyphs::IconSet;
use serde::Deserialize;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The program and leading args that start the agent. Dispatch appends its own
/// args after these. The program is separate from the args, so an empty
/// command can not exist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl AgentCommand {
    /// The full argv for a start or resume.
    pub fn argv(&self, tail: &[String]) -> Vec<String> {
        let mut argv = vec![self.program.clone()];
        argv.extend(self.args.iter().cloned());
        argv.extend(tail.iter().cloned());
        argv
    }

    /// The config boundary: an argv with a program and no blank program name.
    pub fn from_argv(argv: Vec<String>) -> Result<AgentCommand, ConfigError> {
        let mut parts = argv.into_iter();
        let program = parts.next().ok_or(ConfigError::BadCommand("is empty"))?;
        if program.trim().is_empty() {
            return Err(ConfigError::BadCommand("starts with an empty program name"));
        }
        Ok(AgentCommand { program, args: parts.collect() })
    }

    /// One line of words, for editing. Split on whitespace. `'...'` and
    /// `"..."` group words, and there are no escapes.
    pub fn from_words(text: &str) -> Result<AgentCommand, ConfigError> {
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
        AgentCommand::from_argv(argv)
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Multiplexer {
    #[default]
    Auto,
    Tmux,
    Herdr,
    #[serde(rename = "herdr-app")]
    HerdrApp,
}

impl Multiplexer {
    pub fn next(self) -> Multiplexer {
        match self {
            Multiplexer::Auto => Multiplexer::Tmux,
            Multiplexer::Tmux => Multiplexer::Herdr,
            Multiplexer::Herdr => Multiplexer::HerdrApp,
            Multiplexer::HerdrApp => Multiplexer::Auto,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Multiplexer::Auto => "auto",
            Multiplexer::Tmux => "tmux",
            Multiplexer::Herdr => "herdr",
            Multiplexer::HerdrApp => "herdr-app",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(from = "bool")]
pub enum RemoveOnMerge {
    #[default]
    Off,
    On,
}

impl From<bool> for RemoveOnMerge {
    fn from(remove: bool) -> Self {
        if remove { RemoveOnMerge::On } else { RemoveOnMerge::Off }
    }
}

impl RemoveOnMerge {
    pub fn toggle(self) -> RemoveOnMerge {
        match self {
            RemoveOnMerge::Off => RemoveOnMerge::On,
            RemoveOnMerge::On => RemoveOnMerge::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RemoveOnMerge::Off => "off",
            RemoveOnMerge::On => "on",
        }
    }

    pub fn as_bool(self) -> bool {
        self == RemoveOnMerge::On
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub github_refresh: Duration,
    /// On by default. Sessions get Claude hooks that flag "waiting for you"
    /// on their tmux window (see `session::mux::WAITING_HOOKS`). Turn it off if the
    /// `--settings` hooks interfere with the user's own hooks.
    pub detect_waiting: bool,
    pub claude_command: AgentCommand,
    /// Which glyph set marks states. `NerdFont` needs a Nerd Font in the terminal.
    pub icons: IconSet,
    pub multiplexer: Multiplexer,
    pub remove_on_merge: RemoveOnMerge,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            github_refresh: Duration::from_mins(1),
            detect_waiting: true,
            claude_command: AgentCommand { program: "claude".to_string(), args: Vec::new() },
            icons: IconSet::default(),
            multiplexer: Multiplexer::default(),
            remove_on_merge: RemoveOnMerge::default(),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Read { path: PathBuf, source: std::io::Error },
    Json { path: PathBuf, message: String },
    BadCommand(&'static str),
    Write { path: PathBuf, source: std::io::Error },
    NoConfigDir,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            ConfigError::Json { path, message } => write!(f, "invalid config {}: {message}", path.display()),
            ConfigError::BadCommand(why) => write!(f, "invalid config: claude_command {why}"),
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
    /// Removed with the classic layout. Old files that still set it load,
    /// and the value is ignored.
    #[serde(rename = "split")]
    _split: Option<serde::de::IgnoredAny>,
    icons: Option<IconSet>,
    multiplexer: Option<Multiplexer>,
    remove_on_merge: Option<RemoveOnMerge>,
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
        config.claude_command = AgentCommand::from_argv(argv)?;
    }
    if let Some(icons) = file.icons {
        config.icons = icons;
    }
    if let Some(multiplexer) = file.multiplexer {
        config.multiplexer = multiplexer;
    }
    if let Some(remove_on_merge) = file.remove_on_merge {
        config.remove_on_merge = remove_on_merge;
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

#[expect(
    clippy::disallowed_methods,
    reason = "boundary: the config file location comes from XDG_CONFIG_HOME and HOME; `path` is the pure part"
)]
pub(super) fn env_path() -> Option<PathBuf> {
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    path(xdg.as_deref(), home.as_deref())
}

/// The file's text, or `None` when there is no file.
pub(super) fn read(path: &Path) -> Result<Option<String>, ConfigError> {
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
        assert_eq!(c.icons, IconSet::Unicode);
        assert_eq!(c.remove_on_merge, RemoveOnMerge::Off);
    }

    #[test]
    fn empty_object_is_the_default() {
        let c = parse_str("{}").expect("valid");
        assert!(c.detect_waiting);
        assert_eq!(c.claude_command, Config::default().claude_command);
        assert_eq!(c.remove_on_merge, RemoveOnMerge::Off);
    }

    #[test]
    fn fields_replace_defaults() {
        let c = parse_str(
            r#"{"detect_waiting":false,"claude_command":["sh","-c","echo","_"],"icons":"nerdfont","remove_on_merge":true}"#,
        )
        .expect("valid");
        assert!(!c.detect_waiting);
        assert_eq!(c.claude_command.program, "sh");
        assert_eq!(c.claude_command.args, ["-c", "echo", "_"]);
        assert_eq!(c.icons, IconSet::NerdFont);
        assert_eq!(c.remove_on_merge, RemoveOnMerge::On);
    }

    #[test]
    fn invalid_files_are_errors() {
        assert!(matches!(parse_str("not json"), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"skills":[]}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"claude_command":[]}"#), Err(ConfigError::BadCommand(_))));
        assert!(matches!(parse_str(r#"{"claude_command":[" "]}"#), Err(ConfigError::BadCommand(_))));
        assert!(matches!(parse_str(r#"{"detect_waiting":"yes"}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"icons":"emoji"}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"remove_on_merge":"yes"}"#), Err(ConfigError::Json { .. })));
    }

    #[test]
    fn an_old_split_field_loads_and_is_ignored() {
        for text in [r#"{"split":42}"#, r#"{"split":95}"#, r#"{"split":"42"}"#, r#"{"split":null}"#] {
            assert_eq!(parse_str(text).expect("loads"), Config::default(), "{text}");
        }
        let c = parse_str(r#"{"split":30,"icons":"nerdfont"}"#).expect("loads");
        assert_eq!(c.icons, IconSet::NerdFont);
    }

    #[test]
    fn path_prefers_xdg_then_home() {
        assert_eq!(path(Some("/x"), Some("/h")), Some(PathBuf::from("/x/dispatch/config.json")));
        assert_eq!(path(Some(""), Some("/h")), Some(PathBuf::from("/h/.config/dispatch/config.json")));
        assert_eq!(path(None, None), None);
    }

    #[test]
    fn words_and_from_words_round_trip() {
        for argv in [
            vec!["claude"],
            vec!["sh", "-c", "echo hi"],
            vec!["a", "", "b c"],
            vec!["x", "it's", "say \"hi\"", "both ' and \" here"],
        ] {
            let command = AgentCommand::from_argv(argv.iter().map(ToString::to_string).collect()).expect("valid");
            let again = AgentCommand::from_words(&command.words()).expect("valid");
            assert_eq!(again, command, "{}", command.words());
        }
        assert_eq!(AgentCommand::from_words("sh -c 'echo hi'").expect("valid").args, ["-c", "echo hi"]);
        assert_eq!(AgentCommand::from_words("a\"b c\"d").expect("valid").program, "ab cd");
    }

    #[test]
    fn words_reject_an_empty_or_blank_program_or_an_open_quote() {
        for text in ["", "   ", "''", "' ' x", "claude 'oops"] {
            assert!(matches!(AgentCommand::from_words(text), Err(ConfigError::BadCommand(_))), "{text:?}");
        }
    }

    #[test]
    fn multiplexer_labels_match_the_file_spelling() {
        for choice in [Multiplexer::Auto, Multiplexer::Tmux, Multiplexer::Herdr, Multiplexer::HerdrApp] {
            let parsed: Multiplexer = serde_json::from_str(&format!("\"{}\"", choice.label())).expect("parses");
            assert_eq!(parsed, choice);
            assert_eq!(choice.next().next().next().next(), choice);
        }
        assert_eq!(parse_str("{}").expect("loads").multiplexer, Multiplexer::Auto);
        assert!(matches!(parse_str(r#"{"multiplexer":"screen"}"#), Err(ConfigError::Json { .. })));
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
