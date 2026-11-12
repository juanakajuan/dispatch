//! PROTOTYPE: what Claude last said in a worktree, read from its transcript
//! at `~/.claude/projects/<encoded worktree>/<session>.jsonl`. The newest
//! of two entries wins: a recap (`system`/`away_summary`, written when the
//! user steps away) or the text of an assistant message.

use serde::Deserialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Bytes read from the end of the transcript. Transcripts grow to megabytes.
const TAIL_BYTES: u64 = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummaryKind {
    Recap,
    Message,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub kind: SummaryKind,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Entry {
    #[serde(rename = "system")]
    System { subtype: Option<String>, content: Option<String> },
    #[serde(rename = "assistant")]
    Assistant {
        message: Message,
        #[serde(rename = "isSidechain", default)]
        is_sidechain: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct Message {
    content: Vec<Block>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Block {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(other)]
    Other,
}

pub fn latest(worktree: &Path) -> Option<Summary> {
    let file = newest_transcript(&projects_dir()?.join(encode(worktree)))?;
    parse_latest(&read_tail(&file)?)
}

fn projects_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".claude"),
    };
    Some(base.join("projects"))
}

/// Claude Code names a project directory after its path, with each
/// character that is not ASCII alphanumeric replaced by `-`.
fn encode(path: &Path) -> String {
    path.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn newest_transcript(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(at, _)| *at)
        .map(|(_, path)| path)
}

fn read_tail(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES))).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Lines that do not parse (the cut first line, unknown shapes) are skipped.
fn parse_latest(jsonl: &str) -> Option<Summary> {
    jsonl.lines().rev().find_map(|line| match serde_json::from_str::<Entry>(line).ok()? {
        Entry::System { subtype: Some(s), content: Some(text) } if s == "away_summary" => {
            let text = text.trim_end_matches("(disable recaps in /config)").trim().to_string();
            Some(Summary { kind: SummaryKind::Recap, text })
        }
        Entry::Assistant { message, is_sidechain: false } => {
            let text: Vec<String> = message
                .content
                .into_iter()
                .filter_map(|b| match b {
                    Block::Text { text } if !text.trim().is_empty() => Some(text),
                    _ => None,
                })
                .collect();
            (!text.is_empty()).then(|| Summary { kind: SummaryKind::Message, text: text.join(" ") })
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_recap_or_assistant_text_wins_and_tool_calls_are_skipped() {
        let jsonl = [
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Old answer."}]}}"#,
            r#"{"type":"system","subtype":"away_summary","content":"You fixed X. Next, run Y. (disable recaps in /config)"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"1","name":"Bash","input":{}}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"subagent"}]}}"#,
        ]
        .join("\n");
        assert_eq!(
            parse_latest(&jsonl),
            Some(Summary { kind: SummaryKind::Recap, text: "You fixed X. Next, run Y.".to_string() })
        );
        let newer =
            format!("{jsonl}\n{}", r#"{"type":"assistant","message":{"content":[{"type":"text","text":"New."}]}}"#);
        assert_eq!(parse_latest(&newer), Some(Summary { kind: SummaryKind::Message, text: "New.".to_string() }));
    }

    #[test]
    fn worktree_paths_encode_like_claude_code() {
        assert_eq!(
            encode(Path::new("/home/u/Projects/dispatch/.claude/worktrees/35")),
            "-home-u-Projects-dispatch--claude-worktrees-35"
        );
    }
}
