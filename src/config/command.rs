use super::{ConfigError, Harness};

/// The program and leading args that start the agent. Dispatch appends its own
/// args after these. The program is separate from the args, so an empty
/// command can not exist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl AgentCommand {
    /// The harness's own program with no args.
    pub fn default_for(harness: Harness) -> AgentCommand {
        AgentCommand { program: harness.program().to_string(), args: Vec::new() }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
