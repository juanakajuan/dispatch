//! The one place that spawns a subprocess and captures its output.
//! All args are argv elements. Nothing goes through a shell.

use std::ffi::OsStr;
use std::path::Path;

#[derive(Debug)]
pub enum ProcError {
    /// Program missing or not executable. The message names the program, so
    /// "tmux is not installed" reaches the user as is.
    Spawn { program: String, source: std::io::Error },
    Exit { program: String, code: Option<i32>, stderr: String },
    NotUtf8 { program: String },
}

impl ProcError {
    /// For callers that treat one exit code as an empty result
    /// (`git config --get-regexp` exits 1 when nothing matches).
    pub fn exit_code(&self) -> Option<i32> {
        todo!()
    }
}

pub fn run<I, S>(program: &str, args: I, cwd: &Path) -> Result<String, ProcError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    // TODO: Command::new(program).args(args).current_dir(cwd).stdin(Stdio::null())
    //       .output(); map spawn error; non-zero -> Exit with trimmed stderr;
    //       String::from_utf8 -> NotUtf8.
    todo!()
}
