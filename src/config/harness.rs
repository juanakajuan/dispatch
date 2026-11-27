//! The agent program a session runs. One table: every fact that differs by
//! harness is a method here, and `session::start_argv` holds the argv.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    #[default]
    Claude,
    Codex,
    #[serde(rename = "opencode")]
    OpenCode,
    Antigravity,
    Pi,
}

impl Harness {
    const ALL: [Harness; 5] = [Harness::Claude, Harness::Codex, Harness::OpenCode, Harness::Antigravity, Harness::Pi];

    pub fn next(self) -> Harness {
        match self {
            Harness::Claude => Harness::Codex,
            Harness::Codex => Harness::OpenCode,
            Harness::OpenCode => Harness::Antigravity,
            Harness::Antigravity => Harness::Pi,
            Harness::Pi => Harness::Claude,
        }
    }

    pub fn prev(self) -> Harness {
        match self {
            Harness::Claude => Harness::Pi,
            Harness::Codex => Harness::Claude,
            Harness::OpenCode => Harness::Codex,
            Harness::Antigravity => Harness::OpenCode,
            Harness::Pi => Harness::Antigravity,
        }
    }

    /// The file spelling, also the `dispatch-harness` marker value.
    pub fn label(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::OpenCode => "opencode",
            Harness::Antigravity => "antigravity",
            Harness::Pi => "pi",
        }
    }

    /// The inverse of `label`. `None` for a spelling dispatch does not know.
    pub fn from_label(label: &str) -> Option<Harness> {
        Harness::ALL.into_iter().find(|h| h.label() == label)
    }

    /// For messages.
    pub fn name(self) -> &'static str {
        match self {
            Harness::Claude => "Claude",
            Harness::Codex => "Codex",
            Harness::OpenCode => "OpenCode",
            Harness::Antigravity => "Antigravity",
            Harness::Pi => "Pi",
        }
    }

    /// The program that starts it when the user sets no command.
    pub fn program(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
            Harness::OpenCode => "opencode",
            Harness::Antigravity => "agy",
            Harness::Pi => "pi",
        }
    }

    /// Only Claude takes the `--settings` hooks that flag "waiting for you".
    pub fn takes_waiting_hooks(self) -> bool {
        match self {
            Harness::Claude => true,
            Harness::Codex | Harness::OpenCode | Harness::Antigravity | Harness::Pi => false,
        }
    }

    /// Only Claude writes the transcript that `transcript.rs` reads.
    pub fn has_transcript(self) -> bool {
        match self {
            Harness::Claude => true,
            Harness::Codex | Harness::OpenCode | Harness::Antigravity | Harness::Pi => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_match_the_file_spelling_and_cycle() {
        for harness in Harness::ALL {
            let parsed: Harness = serde_json::from_str(&format!("\"{}\"", harness.label())).expect("parses");
            assert_eq!(parsed, harness);
            assert_eq!(Harness::from_label(harness.label()), Some(harness));
            assert_eq!(harness.next().next().next().next().next(), harness);
            assert_eq!(harness.next().prev(), harness);
        }
        assert_eq!(Harness::from_label("gemini"), None);
        assert_eq!(Harness::default(), Harness::Claude);
    }

    #[test]
    fn only_claude_has_hooks_and_a_transcript() {
        let with_hooks: Vec<_> = Harness::ALL.into_iter().filter(|h| h.takes_waiting_hooks()).collect();
        let with_transcript: Vec<_> = Harness::ALL.into_iter().filter(|h| h.has_transcript()).collect();
        assert_eq!(with_hooks, [Harness::Claude]);
        assert_eq!(with_transcript, [Harness::Claude]);
    }
}
