//! Optional user config at `$XDG_CONFIG_HOME/dispatch/config.toml`.
//! A missing file gives `Config::default()`. A present but invalid file is a
//! startup error with the line, not a silent fallback.
//!
//! ```toml
//! skills = ["pstack:tdd", "pstack:architect", "pstack:technical-writing", "pstack:figure-it-out"]
//!
//! [[label_rule]]
//! label = "bug"
//! skill = "pstack:tdd"
//! ```

use crate::domain::SkillName;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct LabelRule {
    pub label: String,
    pub skill: SkillName,
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Ordered. First match wins.
    pub label_rules: Vec<LabelRule>,
    /// What the skill selector cycles through, after "automatic".
    /// Invariant: contains every `label_rules[i].skill` (checked in `load`),
    /// so a default the dialog picks is always selectable.
    pub skills: Vec<SkillName>,
    pub github_refresh: Duration,
    /// On by default. Sessions get Claude hooks that flag "waiting for you"
    /// on their tmux window (see `session::WAITING_HOOKS`). Turn it off if the
    /// `--settings` hooks interfere with the user's own hooks.
    pub detect_waiting: bool,
}

impl Default for Config {
    fn default() -> Config {
        // TODO: bug -> pstack:tdd, feature -> pstack:architect, enhancement -> pstack:architect,
        //       documentation -> pstack:technical-writing, docs -> pstack:technical-writing.
        //       skills = the distinct rule skills + pstack:figure-it-out.
        //       github_refresh = 60s, detect_waiting = true.
        todo!()
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Read(std::io::Error),
    Toml(String),
    BadSkill { value: String },
}

pub fn load() -> Result<Config, ConfigError> {
    // TODO: path from XDG_CONFIG_HOME or HOME/.config. NotFound -> Ok(default).
    //       Private `ConfigFile` serde struct with all fields optional; each present
    //       field replaces the default one; parse skills via SkillName::parse;
    //       append rule skills missing from `skills`.
    todo!()
}
