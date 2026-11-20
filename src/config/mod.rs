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
//!   "harness": "claude",
//!   "command": ["claude"],
//!   "icons": "unicode",
//!   "multiplexer": "auto",
//!   "remove_on_merge": false
//! }
//! ```
//!
//! `harness` is `claude`, `codex`, `opencode`, or `antigravity`. `command` is
//! the program and leading args for that harness, and its default is the
//! harness's own program. The old `claude_command` key still loads, as
//! `"harness": "claude"` with that command.

mod command;
pub mod harness;
pub mod settings;

pub use command::AgentCommand;
pub use harness::Harness;

use crate::glyphs::IconSet;
use serde::Deserialize;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    /// The agent that new sessions run.
    pub harness: Harness,
    /// Replaces the program of `harness`. `None` runs the harness's own program.
    pub command: Option<AgentCommand>,
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
            harness: Harness::default(),
            command: None,
            icons: IconSet::default(),
            multiplexer: Multiplexer::default(),
            remove_on_merge: RemoveOnMerge::default(),
        }
    }
}

impl Config {
    /// The command that starts `harness`. The `command` override belongs to
    /// the configured harness only, so a session of another harness never
    /// gets its args.
    pub fn agent_command(&self, harness: Harness) -> AgentCommand {
        match &self.command {
            Some(command) if harness == self.harness => command.clone(),
            _ => AgentCommand::default_for(harness),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    Json {
        path: PathBuf,
        message: String,
    },
    BadCommand(&'static str),
    /// Two keys that cannot both be set.
    Conflict(&'static str),
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    NoConfigDir,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            ConfigError::Json { path, message } => write!(f, "invalid config {}: {message}", path.display()),
            ConfigError::BadCommand(why) => write!(f, "invalid config: command {why}"),
            ConfigError::Conflict(why) => write!(f, "invalid config: {why}"),
            ConfigError::Write { path, source } => write!(f, "cannot write {}: {source}", path.display()),
            ConfigError::NoConfigDir => f.write_str("cannot save: neither XDG_CONFIG_HOME nor HOME is set"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    detect_waiting: Option<bool>,
    harness: Option<Harness>,
    command: Option<Vec<String>>,
    /// The old spelling of `command` for Claude.
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
    if let Some(harness) = file.harness {
        config.harness = harness;
    }
    let command = match (file.command, file.claude_command) {
        (Some(_), Some(_)) => {
            return Err(ConfigError::Conflict("claude_command and command are both set; keep command"));
        }
        (None, Some(_)) if config.harness != Harness::Claude => {
            return Err(ConfigError::Conflict("claude_command only works with the claude harness; use command"));
        }
        (command, claude_command) => command.or(claude_command),
    };
    config.command = command.map(AgentCommand::from_argv).transpose()?;
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
        assert_eq!(c.harness, Harness::Claude);
        assert_eq!(c.agent_command(c.harness).argv(&["x".to_string()]), ["claude", "x"]);
        assert_eq!(c.icons, IconSet::Unicode);
        assert_eq!(c.remove_on_merge, RemoveOnMerge::Off);
    }

    #[test]
    fn empty_object_is_the_default() {
        let c = parse_str("{}").expect("valid");
        assert!(c.detect_waiting);
        assert_eq!(c, Config::default());
        assert_eq!(c.remove_on_merge, RemoveOnMerge::Off);
    }

    #[test]
    fn fields_replace_defaults() {
        let c = parse_str(
            r#"{"detect_waiting":false,"command":["sh","-c","echo","_"],"icons":"nerdfont","remove_on_merge":true}"#,
        )
        .expect("valid");
        assert!(!c.detect_waiting);
        let command = c.command.expect("override");
        assert_eq!(command.program, "sh");
        assert_eq!(command.args, ["-c", "echo", "_"]);
        assert_eq!(c.icons, IconSet::NerdFont);
        assert_eq!(c.remove_on_merge, RemoveOnMerge::On);
    }

    #[test]
    fn invalid_files_are_errors() {
        assert!(matches!(parse_str("not json"), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"skills":[]}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"command":[]}"#), Err(ConfigError::BadCommand(_))));
        assert!(matches!(parse_str(r#"{"claude_command":[]}"#), Err(ConfigError::BadCommand(_))));
        assert!(matches!(parse_str(r#"{"command":[" "]}"#), Err(ConfigError::BadCommand(_))));
        assert!(matches!(parse_str(r#"{"detect_waiting":"yes"}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"harness":"gemini"}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"icons":"emoji"}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"remove_on_merge":"yes"}"#), Err(ConfigError::Json { .. })));
    }

    #[test]
    fn harness_and_command_load() {
        let c = parse_str(r#"{"harness":"codex","command":["my-codex","--x"]}"#).expect("loads");
        assert_eq!(c.harness, Harness::Codex);
        assert_eq!(c.agent_command(Harness::Codex).argv(&[]), ["my-codex", "--x"]);
        assert_eq!(parse_str(r#"{"harness":"opencode"}"#).expect("loads").command, None);
    }

    #[test]
    fn the_command_override_belongs_to_the_configured_harness() {
        let c = parse_str(r#"{"harness":"codex","command":["my-codex"]}"#).expect("loads");
        assert_eq!(c.agent_command(Harness::Claude).argv(&[]), ["claude"]);
        assert_eq!(c.agent_command(Harness::Antigravity).argv(&[]), ["agy"]);
    }

    #[test]
    fn the_old_claude_command_key_still_loads_as_claude() {
        let c = parse_str(r#"{"claude_command":["sh","-c","echo","_"]}"#).expect("loads");
        assert_eq!(c.harness, Harness::Claude);
        assert_eq!(c.agent_command(Harness::Claude).argv(&[]), ["sh", "-c", "echo", "_"]);
        let c = parse_str(r#"{"harness":"claude","claude_command":["my-claude"]}"#).expect("loads");
        assert_eq!(c.agent_command(Harness::Claude).program, "my-claude");
    }

    #[test]
    fn claude_command_conflicts_are_errors() {
        for text in [r#"{"claude_command":["a"],"command":["b"]}"#, r#"{"harness":"codex","claude_command":["a"]}"#] {
            assert!(matches!(parse_str(text), Err(ConfigError::Conflict(_))), "{text}");
        }
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
