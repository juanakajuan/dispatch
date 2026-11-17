//! What Claude last said in a worktree, read from its transcript at
//! `~/.claude/projects/<encoded worktree>/<session>.jsonl`. The newest of two
//! entries wins: a recap (`system`/`away_summary`, written when the user
//! steps away) or the text of an assistant message.

use crate::domain::IssueNumber;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

/// A conversation Claude saved in a worktree. `claude --continue` needs one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    /// None until Claude says something.
    pub said: Option<Summary>,
}

/// The size and mtime of a transcript file when it was last read. A file
/// with the same stamp is not read again.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: SystemTime,
}

/// Reads the transcripts of the sessions on each poll. It keeps what it read
/// last, by file, and reads a file again only when its stamp changed.
#[derive(Default)]
pub struct Reader {
    seen: BTreeMap<PathBuf, (Stamp, Option<Summary>)>,
}

impl Reader {
    /// One entry for each worktree that has a conversation. Files of
    /// worktrees not in this pass are forgotten.
    pub fn read<'a>(
        &mut self,
        worktrees: impl IntoIterator<Item = (IssueNumber, &'a Path)>,
    ) -> BTreeMap<IssueNumber, Transcript> {
        let Some(projects) = projects_dir() else { return BTreeMap::new() };
        self.read_in(&projects, worktrees)
    }

    fn read_in<'a>(
        &mut self,
        projects: &Path,
        worktrees: impl IntoIterator<Item = (IssueNumber, &'a Path)>,
    ) -> BTreeMap<IssueNumber, Transcript> {
        let mut seen = BTreeMap::new();
        let mut out = BTreeMap::new();
        for (issue, worktree) in worktrees {
            let Some((file, stamp)) = newest_transcript(&projects.join(encode(worktree))) else { continue };
            let said = match self.seen.remove(&file) {
                Some((old, said)) if old == stamp => said,
                _ => read_tail(&file).and_then(|text| parse_latest(&text)),
            };
            out.insert(issue, Transcript { said: said.clone() });
            seen.insert(file, (stamp, said));
        }
        self.seen = seen;
        out
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "boundary: Claude Code keeps its transcripts under CLAUDE_CONFIG_DIR or HOME"
)]
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

fn newest_transcript(dir: &Path) -> Option<(PathBuf, Stamp)> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((e.path(), Stamp { len: meta.len(), modified: meta.modified().ok()? }))
        })
        .max_by_key(|(_, stamp)| stamp.modified)
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

    fn assistant(text: &str) -> String {
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":text}]}}).to_string()
    }

    #[test]
    fn the_text_blocks_of_one_message_join_and_blank_ones_are_skipped() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"First."},{"type":"thinking","thinking":"hm"},{"type":"text","text":"  "},{"type":"text","text":"Second."}]}}"#;
        assert_eq!(
            parse_latest(line),
            Some(Summary { kind: SummaryKind::Message, text: "First. Second.".to_string() })
        );
    }

    #[test]
    fn a_cut_line_user_turns_and_other_system_entries_say_nothing() {
        let jsonl = [
            r#"ntent":[{"type":"text","text":"cut by the tail read"}]}}"#,
            r#"{"type":"user","message":{"content":"Fix it."}}"#,
            r#"{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}"#,
            r#"{"type":"summary","summary":"Title"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"1","name":"Read","input":{}}]}}"#,
        ]
        .join("\n");
        assert_eq!(parse_latest(&jsonl), None);
        let with_answer = format!("{}\n{jsonl}", assistant("Done."));
        assert_eq!(parse_latest(&with_answer).map(|s| s.text), Some("Done.".to_string()));
    }

    fn said(reader: &mut Reader, projects: &Path, worktree: &Path) -> Option<String> {
        let mut read = reader.read_in(projects, [(IssueNumber(7), worktree)]);
        read.remove(&IssueNumber(7)).and_then(|t| t.said).map(|s| s.text)
    }

    #[test]
    fn a_transcript_is_read_again_only_when_its_size_or_mtime_changed() {
        let projects = crate::testutil::temp_dir("transcript-stamp");
        let worktree = Path::new("/w/orbit-7");
        let dir = projects.join(encode(worktree));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s.jsonl");
        std::fs::write(&file, assistant("Old.")).unwrap();
        let mtime = std::fs::metadata(&file).unwrap().modified().unwrap();
        let mut reader = Reader::default();
        assert_eq!(said(&mut reader, &projects, worktree), Some("Old.".to_string()));

        std::fs::write(&file, assistant("New.")).unwrap();
        File::options().write(true).open(&file).unwrap().set_modified(mtime).unwrap();
        assert_eq!(said(&mut reader, &projects, worktree), Some("Old.".to_string()), "same size and mtime: not read");

        std::fs::write(&file, format!("{}\n{}", assistant("New."), assistant("Newer."))).unwrap();
        assert_eq!(said(&mut reader, &projects, worktree), Some("Newer.".to_string()), "a new size: read again");

        assert!(reader.read_in(&projects, []).is_empty());
        assert!(reader.seen.is_empty(), "a worktree that is gone is forgotten");
    }

    #[test]
    fn the_newest_transcript_wins_and_only_its_tail_is_read() {
        let projects = crate::testutil::temp_dir("transcript-tail");
        let worktree = Path::new("/w/orbit-8");
        let dir = projects.join(encode(worktree));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old.jsonl");
        std::fs::write(&old, assistant("Old session.")).unwrap();
        let earlier = std::fs::metadata(&old).unwrap().modified().unwrap() - std::time::Duration::from_mins(1);
        File::options().write(true).open(&old).unwrap().set_modified(earlier).unwrap();
        let big = usize::try_from(TAIL_BYTES).unwrap();
        let padding = assistant(&"x".repeat(big));
        std::fs::write(dir.join("new.jsonl"), format!("{padding}\n{}", assistant("Last."))).unwrap();
        assert_eq!(said(&mut Reader::default(), &projects, worktree), Some("Last.".to_string()));
        assert!(said(&mut Reader::default(), &projects, Path::new("/w/none")).is_none(), "no conversation");
    }

    #[test]
    fn worktree_paths_encode_like_claude_code() {
        assert_eq!(
            encode(Path::new("/home/u/Projects/dispatch/.claude/worktrees/35")),
            "-home-u-Projects-dispatch--claude-worktrees-35"
        );
    }
}
