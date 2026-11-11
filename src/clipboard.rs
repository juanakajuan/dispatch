//! Copy text to the clipboard of the terminal the user looks at, which over
//! SSH is on the local machine. Inside tmux, `set-buffer -w` makes tmux send
//! OSC 52 to its client, and it does so with tmux's default `set-clipboard`.
//! Outside tmux, dispatch writes OSC 52 itself.

use crate::proc;
use std::io::Write;
use std::path::Path;

/// `inside_tmux` is `$TMUX` being set. Call it on the main thread only: a
/// write from another thread can land inside a frame's escape sequence.
pub fn copy(text: &str, inside_tmux: bool, cwd: &Path) -> Result<(), String> {
    if inside_tmux {
        return proc::run("tmux", ["set-buffer", "-w", text], cwd).map(|_| ()).map_err(|e| e.to_string());
    }
    let mut out = std::io::stdout().lock();
    out.write_all(osc52(text).as_bytes()).and_then(|()| out.flush()).map_err(|e| e.to_string())
}

fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[usize::try_from((n >> shift) & 63).unwrap_or(0)]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        let cases =
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE=")];
        for (input, want) in cases {
            assert_eq!(base64(input.as_bytes()), want, "{input:?}");
        }
    }

    #[test]
    fn osc52_sets_the_clipboard_selection() {
        assert_eq!(osc52("foo"), "\x1b]52;c;Zm9v\x07");
    }

    #[test]
    fn inside_tmux_copies_with_set_buffer_w() {
        proc::fake::install(|_| Ok(String::new()));
        copy("https://github.com/o/r/issues/2", true, Path::new("/r")).expect("copies");
        assert_eq!(proc::fake::calls(), ["tmux set-buffer -w https://github.com/o/r/issues/2"]);
    }
}
