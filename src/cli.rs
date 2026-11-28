//! The command line. dispatch takes no arguments to run; `--help` and
//! `--version` print and exit, and any other argument is an error.

use std::ffi::OsString;
use std::fmt;
use std::path::Path;

/// What the command line asks dispatch to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    Run,
    Help,
    Version,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliError {
    Unknown(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Unknown(arg) => write!(f, "unknown argument `{arg}`. Run `dispatch --help` for usage."),
        }
    }
}

/// Reads the arguments after the program name. The first of `--help` and
/// `--version` wins, but only when every argument is known.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Invocation, CliError> {
    let mut invocation = Invocation::Run;
    for arg in args {
        let known = match arg.to_str() {
            Some("-h" | "--help") => Invocation::Help,
            Some("-V" | "--version") => Invocation::Version,
            _ => return Err(CliError::Unknown(arg.to_string_lossy().into_owned())),
        };
        if invocation == Invocation::Run {
            invocation = known;
        }
    }
    Ok(invocation)
}

pub fn version() -> String {
    format!("dispatch {}", env!("CARGO_PKG_VERSION"))
}

/// `config` is `None` when neither `XDG_CONFIG_HOME` nor `HOME` is set.
pub fn help(config: Option<&Path>) -> String {
    let config = match config {
        Some(path) => path.display().to_string(),
        None => "none, because neither XDG_CONFIG_HOME nor HOME is set".to_owned(),
    };
    format!(
        "dispatch: a terminal app that lists a GitHub repo's issues and pull requests and runs an agent session for each.

Usage: dispatch [OPTIONS]

Run it inside a git checkout that has a GitHub remote called `origin`.
Press ? in the app to see the keys.

Options:
  -h, --help     Print this help and exit
  -V, --version  Print the version and exit

Config: {config}
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<Invocation, CliError> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn no_arguments_runs_the_app() {
        assert_eq!(parse_strs(&[]), Ok(Invocation::Run));
    }

    #[test]
    fn help_and_version_have_a_short_and_a_long_form() {
        assert_eq!(parse_strs(&["-h"]), Ok(Invocation::Help));
        assert_eq!(parse_strs(&["--help"]), Ok(Invocation::Help));
        assert_eq!(parse_strs(&["-V"]), Ok(Invocation::Version));
        assert_eq!(parse_strs(&["--version"]), Ok(Invocation::Version));
    }

    #[test]
    fn the_first_known_argument_wins() {
        assert_eq!(parse_strs(&["--version", "--help"]), Ok(Invocation::Version));
        assert_eq!(parse_strs(&["-h", "-V"]), Ok(Invocation::Help));
    }

    #[test]
    fn an_unknown_argument_is_an_error_even_after_a_known_one() {
        assert_eq!(parse_strs(&["--verison"]), Err(CliError::Unknown("--verison".to_owned())));
        assert_eq!(parse_strs(&["--help", "extra"]), Err(CliError::Unknown("extra".to_owned())));
    }

    #[test]
    fn an_unknown_argument_keeps_its_sentence() {
        assert_eq!(
            CliError::Unknown("--nope".to_owned()).to_string(),
            "unknown argument `--nope`. Run `dispatch --help` for usage."
        );
    }

    #[test]
    fn the_version_is_the_crate_version() {
        assert_eq!(version(), format!("dispatch {}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn help_names_the_options_and_the_config_file() {
        let text = help(Some(Path::new("/home/me/.config/dispatch/config.json")));
        assert!(text.starts_with("dispatch: "));
        assert!(text.contains("Usage: dispatch [OPTIONS]"));
        assert!(text.contains("-h, --help"));
        assert!(text.contains("-V, --version"));
        assert!(text.contains("Config: /home/me/.config/dispatch/config.json\n"));
    }

    #[test]
    fn help_says_why_there_is_no_config_file() {
        assert!(help(None).contains("Config: none, because neither XDG_CONFIG_HOME nor HOME is set\n"));
    }
}
