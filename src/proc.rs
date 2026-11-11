//! The one place that spawns a subprocess and captures its output.
//! All args are argv elements. Nothing goes through a shell.

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::process::{Command, Stdio};

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
        match self {
            ProcError::Exit { code, .. } => *code,
            _ => None,
        }
    }

    /// The stderr text of a failed exit, empty for the other variants.
    pub fn stderr(&self) -> &str {
        match self {
            ProcError::Exit { stderr, .. } => stderr,
            _ => "",
        }
    }
}

impl fmt::Display for ProcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcError::Spawn { program, source } => write!(f, "cannot run {program}: {source}"),
            ProcError::Exit { program, code, stderr } => {
                let code = code.map_or_else(|| "a signal".to_string(), |c| format!("code {c}"));
                if stderr.is_empty() {
                    write!(f, "{program} failed with {code}")
                } else {
                    write!(f, "{program} failed with {code}: {stderr}")
                }
            }
            ProcError::NotUtf8 { program } => write!(f, "{program} printed text that is not UTF-8"),
        }
    }
}

pub fn run<I, S>(program: &str, args: I, cwd: &Path) -> Result<String, ProcError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<_> = args.into_iter().collect();
    #[cfg(test)]
    if let Some(result) = fake::intercept(program, &args, cwd) {
        return result;
    }
    let output = Command::new(program)
        .args(&args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|source| ProcError::Spawn { program: program.to_string(), source })?;
    if !output.status.success() {
        return Err(ProcError::Exit {
            program: program.to_string(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    String::from_utf8(output.stdout).map_err(|_| ProcError::NotUtf8 { program: program.to_string() })
}

/// The test seam. A test installs a script for its own thread, and `run`
/// answers from it instead of spawning. Each test runs on its own thread, so
/// scripts never leak between tests.
#[cfg(test)]
pub mod fake {
    use super::ProcError;
    use std::cell::RefCell;
    use std::ffi::OsStr;
    use std::path::Path;

    /// One recorded call: `program arg arg ...`.
    pub type Call = String;
    type Script = Box<dyn FnMut(&str) -> Result<String, ProcError>>;

    thread_local! {
        static SCRIPT: RefCell<Option<Script>> = const { RefCell::new(None) };
        static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
    }

    /// Installs `script` for this thread and clears the call log. The script
    /// gets the call as one string and returns its output.
    pub fn install(script: impl FnMut(&str) -> Result<String, ProcError> + 'static) {
        SCRIPT.with(|s| *s.borrow_mut() = Some(Box::new(script)));
        CALLS.with(|c| c.borrow_mut().clear());
    }

    pub fn calls() -> Vec<Call> {
        CALLS.with(|c| c.borrow().clone())
    }

    pub fn fail(program: &str, code: i32, stderr: &str) -> ProcError {
        ProcError::Exit { program: program.to_string(), code: Some(code), stderr: stderr.to_string() }
    }

    pub(super) fn intercept<S: AsRef<OsStr>>(
        program: &str,
        args: &[S],
        _cwd: &Path,
    ) -> Option<Result<String, ProcError>> {
        let mut call = program.to_string();
        for a in args {
            call.push(' ');
            call.push_str(&a.as_ref().to_string_lossy());
        }
        SCRIPT.with(|s| {
            let mut s = s.borrow_mut();
            let script = s.as_mut()?;
            CALLS.with(|c| c.borrow_mut().push(call.clone()));
            Some(script(&call))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_stdout_and_reports_exit_codes() {
        let out = run("sh", ["-c", "printf hi"], Path::new("/")).expect("ok");
        assert_eq!(out, "hi");
        let err = run("sh", ["-c", "echo bad >&2; exit 3"], Path::new("/")).expect_err("fails");
        assert_eq!(err.exit_code(), Some(3));
        assert_eq!(err.stderr(), "bad");
        assert!(err.to_string().contains("code 3: bad"));
    }

    #[test]
    fn missing_program_names_it() {
        let err = run("dispatch-no-such-program", ["x"], Path::new("/")).expect_err("fails");
        assert!(matches!(err, ProcError::Spawn { .. }));
        assert!(err.to_string().contains("dispatch-no-such-program"));
    }
}
