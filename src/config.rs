//! Optional user config at `$XDG_CONFIG_HOME/dispatch/config.json`, falling
//! back to `~/.config/dispatch/config.json`. A missing file gives
//! `Config::default()`. A present but invalid file is a startup error, not a
//! silent fallback. Every field is optional, and a present field replaces the
//! default one.
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
}

#[derive(Clone, Debug)]
pub struct Config {
    pub github_refresh: Duration,
    /// On by default. Sessions get Claude hooks that flag "waiting for you"
    /// on their tmux window (see `session::WAITING_HOOKS`). Turn it off if the
    /// `--settings` hooks interfere with the user's own hooks.
    pub detect_waiting: bool,
    pub claude_command: ClaudeCommand,
    /// Where the list | detail border starts, and where `=` puts it back.
    /// Resizes stay in memory; dispatch never writes this file.
    pub split: Split,
    /// Which glyph set marks states. `NerdFont` needs a Nerd Font in the terminal.
    pub icons: IconSet,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            github_refresh: Duration::from_secs(60),
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
        let mut parts = argv.into_iter();
        let program = parts.next().ok_or(ConfigError::BadCommand("is empty"))?;
        if program.trim().is_empty() {
            return Err(ConfigError::BadCommand("starts with an empty program name"));
        }
        config.claude_command = ClaudeCommand { program, args: parts.collect() };
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

pub fn load() -> Result<Config, ConfigError> {
    let xdg = std::env::var("XDG_CONFIG_HOME").ok();
    let home = std::env::var("HOME").ok();
    let Some(path) = path(xdg.as_deref(), home.as_deref()) else {
        return Ok(Config::default());
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text, &path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(source) => Err(ConfigError::Read { path, source }),
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
}
