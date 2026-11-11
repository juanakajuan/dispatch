# Open on GitHub

A user presses `o` to open the selected issue or PR in the browser. On the Sessions tab, `o` opens the session's issue. dispatch runs `gh issue view <n> --web` or `gh pr view <n> --web`, so it uses the host that `gh` is logged in to.

Over SSH (`SSH_CONNECTION` or `SSH_TTY` is set), `gh … --web` would open a browser on the remote machine. So `o` copies the page URL to the clipboard of the local terminal instead. dispatch gets the URL with `gh <kind> view <n> --json url --jq .url`. Inside tmux it runs `tmux set-buffer -w <url>`, and tmux sends OSC 52 to its client. Outside tmux, dispatch writes OSC 52 itself.

## Sub-features

- `open-issue` opens the selected issue page.
- `open-pr` opens the selected PR page.
- `open-session` opens the issue of the selected session.
- `open-toast` shows `Opening issue #N on GitHub` or `Opening PR #P on GitHub`.
- `copy-over-ssh` shows `Copying the PR #P URL`, then `Copied <url>`. The toast shows the URL because a terminal can ignore OSC 52 and give no error.

## How to get to it (user POV)

- Select a row on any tab and press `o`.

## Driving it with verify.sh

Preconditions:

- A run is up and doctor shows no `FAIL`. `GH_BROWSER` is `$RUN/bin/browser`, so no browser opens.

- **Issue.** Press `1`, select `#N`, press `o`. Run `$V keys "$RUN" 1 o` and `$V wait "$RUN" 'Opening issue #N on GitHub'`. `$RUN/evidence/browser.log` gets `https://github.com/<owner>/<repo>/issues/N`. `gh-calls.log` has `gh issue view N --web`.
- **PR.** Press `2`, select `#P`, press `o`. `browser.log` gets `…/pull/P`, and `gh-calls.log` has `gh pr view P --web`.
- **Session.** Press `3`, select the `#N` session, press `o`. `browser.log` gets `…/issues/N`.
- **Proof.** Run `$V shot "$RUN" open-toast` after each press, and quote the `browser.log` lines.
- **Copy over SSH.** Start the run with `SSH_CONNECTION='10.0.0.2 5000 10.0.0.1 22' $V up`, then run `$V client "$RUN"`. Press `2`, select `#P`, press `o`, and `$V wait "$RUN" 'Copied https://'`. `$V tmux "$RUN" show-buffer` prints the URL. `gh-calls.log` has `gh pr view P --json url --jq .url`, and `browser.log` gets no line.
- **OSC 52 outside tmux.** Run `$V tmux "$RUN" set-option -g set-clipboard on`. Then start `env -u TMUX -u TMUX_PANE SSH_CONNECTION=… $BIN` in a new window of the run's server (the server's `TMUX_TMPDIR` keeps it off the user's tmux) and press `2 o` there. tmux turns dispatch's own OSC 52 into a buffer, and `show-buffer` prints the URL.

## Gotchas

- `browser.log` gets a line only when gh calls the browser. If the toast shows `failed`, read the gh error in the message row.
- `up` passes your environment to dispatch. If your shell runs over SSH, `o` copies and does not call the browser. Unset `SSH_CONNECTION` and `SSH_TTY` before `up` to test the browser path.
- Outside this sandbox, `o` opens a real browser tab. Do not run `gh … --web` in the user's shell to check a URL.
