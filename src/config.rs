//! Optional user config at `$XDG_CONFIG_HOME/dispatch/config.json`, falling
//! back to `~/.config/dispatch/config.json`. A missing file gives
//! `Config::default()`. A present but invalid file is a startup error, not a
//! silent fallback. Every field is optional, and a present field replaces the
//! default one.
//!
//! ```json
//! {
//!   "skills": ["pstack:tdd", "pstack:architect"],
//!   "label_skills": [{ "label": "bug", "skill": "pstack:tdd" }],
//!   "detect_waiting": true,
//!   "claude_command": ["claude"],
//!   "split": 42
//! }
//! ```

use crate::domain::{SkillName, Split};
use serde::Deserialize;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelRule {
    pub label: String,
    pub skill: SkillName,
}

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
    /// Ordered. First match wins.
    pub label_rules: Vec<LabelRule>,
    /// What the skill selector cycles through, after "automatic".
    /// Invariant: contains every `label_rules[i].skill` (checked in `parse`),
    /// so a default the dialog picks is always selectable.
    pub skills: Vec<SkillName>,
    pub github_refresh: Duration,
    /// On by default. Sessions get Claude hooks that flag "waiting for you"
    /// on their tmux window (see `session::WAITING_HOOKS`). Turn it off if the
    /// `--settings` hooks interfere with the user's own hooks.
    pub detect_waiting: bool,
    pub claude_command: ClaudeCommand,
    /// Where the list | detail border starts, and where `=` puts it back.
    /// Resizes stay in memory; dispatch never writes this file.
    pub split: Split,
}

impl Default for Config {
    fn default() -> Config {
        const RULES: [(&str, &str); 5] = [
            ("bug", "pstack:tdd"),
            ("feature", "pstack:architect"),
            ("enhancement", "pstack:architect"),
            ("documentation", "pstack:technical-writing"),
            ("docs", "pstack:technical-writing"),
        ];
        // The literals are valid, and `default_covers_every_literal` proves it,
        // so `filter_map` drops nothing.
        let label_rules: Vec<LabelRule> = RULES
            .iter()
            .filter_map(|(label, skill)| {
                SkillName::parse(skill).ok().map(|skill| LabelRule { label: (*label).to_string(), skill })
            })
            .collect();
        let mut skills = Vec::new();
        for r in &label_rules {
            push_unique(&mut skills, r.skill.clone());
        }
        if let Ok(s) = SkillName::parse("pstack:figure-it-out") {
            push_unique(&mut skills, s);
        }
        Config {
            label_rules,
            skills,
            github_refresh: Duration::from_secs(60),
            detect_waiting: true,
            claude_command: ClaudeCommand { program: "claude".to_string(), args: Vec::new() },
            split: Split::DEFAULT,
        }
    }
}

fn push_unique(skills: &mut Vec<SkillName>, skill: SkillName) {
    if !skills.contains(&skill) {
        skills.push(skill);
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Read { path: PathBuf, source: std::io::Error },
    Json { path: PathBuf, message: String },
    BadSkill { value: String, reason: String },
    BadCommand(&'static str),
    BadSplit(u64),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            ConfigError::Json { path, message } => write!(f, "invalid config {}: {message}", path.display()),
            ConfigError::BadSkill { value, reason } => write!(f, "invalid config: skill {value:?}: {reason}"),
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
    skills: Option<Vec<String>>,
    label_skills: Option<Vec<WireLabelSkill>>,
    detect_waiting: Option<bool>,
    claude_command: Option<Vec<String>>,
    split: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireLabelSkill {
    label: String,
    skill: String,
}

fn skill(value: &str) -> Result<SkillName, ConfigError> {
    SkillName::parse(value).map_err(|reason| ConfigError::BadSkill { value: value.to_string(), reason })
}

/// Pure: config text to `Config`. `path` only labels errors.
pub fn parse(text: &str, path: &Path) -> Result<Config, ConfigError> {
    let file: ConfigFile = serde_json::from_str(text)
        .map_err(|e| ConfigError::Json { path: path.to_path_buf(), message: e.to_string() })?;
    let mut config = Config::default();
    if let Some(rules) = file.label_skills {
        config.label_rules = rules
            .iter()
            .map(|r| skill(&r.skill).map(|skill| LabelRule { label: r.label.clone(), skill }))
            .collect::<Result<_, _>>()?;
    }
    if let Some(names) = file.skills {
        config.skills = names.iter().map(|n| skill(n)).collect::<Result<_, _>>()?;
    }
    for rule in &config.label_rules {
        push_unique(&mut config.skills, rule.skill.clone());
    }
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
        assert_eq!(c.label_rules.len(), 5);
        assert_eq!(c.skills.len(), 4);
        assert!(c.label_rules.iter().all(|r| c.skills.contains(&r.skill)));
        assert!(c.skills.iter().any(|s| s.as_str() == "pstack:figure-it-out"));
        assert!(c.detect_waiting);
        assert_eq!(c.claude_command.argv(&["x".to_string()]), ["claude", "x"]);
        assert_eq!(c.split, Split::DEFAULT);
    }

    #[test]
    fn empty_object_is_the_default() {
        let c = parse_str("{}").expect("valid");
        assert_eq!(c.skills, Config::default().skills);
        assert_eq!(c.label_rules, Config::default().label_rules);
    }

    #[test]
    fn fields_replace_defaults_and_rule_skills_are_selectable() {
        let c = parse_str(
            r#"{"skills":["a"],"label_skills":[{"label":"x","skill":"b:c"}],
                "detect_waiting":false,"claude_command":["sh","-c","echo","_"],"split":30}"#,
        )
        .expect("valid");
        let names: Vec<_> = c.skills.iter().map(SkillName::as_str).collect();
        assert_eq!(names, ["a", "b:c"]);
        assert_eq!(c.label_rules.len(), 1);
        assert!(!c.detect_waiting);
        assert_eq!(c.claude_command.program, "sh");
        assert_eq!(c.claude_command.args, ["-c", "echo", "_"]);
        assert_eq!(c.split.list_width(100), 30);
    }

    #[test]
    fn invalid_files_are_errors() {
        assert!(matches!(parse_str("not json"), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"skils":[]}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"skills":["a b"]}"#), Err(ConfigError::BadSkill { .. })));
        assert!(matches!(parse_str(r#"{"claude_command":[]}"#), Err(ConfigError::BadCommand(_))));
        assert!(matches!(parse_str(r#"{"claude_command":[" "]}"#), Err(ConfigError::BadCommand(_))));
        assert!(matches!(parse_str(r#"{"detect_waiting":"yes"}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"split":95}"#), Err(ConfigError::BadSplit(95))));
        assert!(matches!(parse_str(r#"{"split":-1}"#), Err(ConfigError::Json { .. })));
        assert!(matches!(parse_str(r#"{"split":"42"}"#), Err(ConfigError::Json { .. })));
    }

    #[test]
    fn path_prefers_xdg_then_home() {
        assert_eq!(path(Some("/x"), Some("/h")), Some(PathBuf::from("/x/dispatch/config.json")));
        assert_eq!(path(Some(""), Some("/h")), Some(PathBuf::from("/h/.config/dispatch/config.json")));
        assert_eq!(path(None, None), None);
    }
}
