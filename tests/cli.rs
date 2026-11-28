//! Runs the built binary, because the arguments are read before any repo,
//! config, or terminal setup, and only the real process shows that.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[expect(clippy::disallowed_methods, reason = "the test runs the real binary, so there is no proc fake to go through")]
fn dispatch(args: &[&str], cwd: &Path) -> io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_dispatch")).args(args).current_dir(cwd).env("XDG_CONFIG_HOME", "/xdg").output()
}

/// An empty directory that is not a git checkout, removed on drop.
struct OutsideRepo(PathBuf);

impl OutsideRepo {
    fn new(test: &str) -> io::Result<OutsideRepo> {
        let path = std::env::temp_dir().join(format!("dispatch-cli-{}-{test}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(OutsideRepo(path))
    }
}

impl Drop for OutsideRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn version_prints_and_exits_zero_in_and_out_of_a_repo() -> io::Result<()> {
    let outside = OutsideRepo::new("version")?;
    for cwd in [outside.0.as_path(), Path::new(env!("CARGO_MANIFEST_DIR"))] {
        for flag in ["--version", "-V"] {
            let out = dispatch(&[flag], cwd)?;
            assert_eq!(out.status.code(), Some(0), "{flag} in {}", cwd.display());
            assert_eq!(String::from_utf8_lossy(&out.stdout), format!("dispatch {}\n", env!("CARGO_PKG_VERSION")));
            assert!(out.stderr.is_empty());
        }
    }
    Ok(())
}

#[test]
fn help_prints_usage_and_the_config_path_and_exits_zero() -> io::Result<()> {
    let outside = OutsideRepo::new("help")?;
    for cwd in [outside.0.as_path(), Path::new(env!("CARGO_MANIFEST_DIR"))] {
        for flag in ["--help", "-h"] {
            let out = dispatch(&[flag], cwd)?;
            assert_eq!(out.status.code(), Some(0), "{flag} in {}", cwd.display());
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(stdout.contains("Usage: dispatch [OPTIONS]"), "{stdout}");
            assert!(stdout.contains("Config: /xdg/dispatch/config.json\n"), "{stdout}");
        }
    }
    Ok(())
}

#[test]
fn an_unknown_argument_is_an_error_with_exit_code_two() -> io::Result<()> {
    let outside = OutsideRepo::new("unknown")?;
    for cwd in [outside.0.as_path(), Path::new(env!("CARGO_MANIFEST_DIR"))] {
        let out = dispatch(&["--verison"], cwd)?;
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            "dispatch: unknown argument `--verison`. Run `dispatch --help` for usage.\n"
        );
    }
    Ok(())
}
