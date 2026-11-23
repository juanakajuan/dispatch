# Open on GitHub

A user presses `o` to open the selected issue or PR in the browser. On the Sessions tab, `o` opens the session's issue, or the PR of a PR session (`PR #P`). dispatch runs `gh issue view <n> --web` or `gh pr view <n> --web`, so it uses the host that `gh` is logged in to.

Over SSH (`SSH_CONNECTION` or `SSH_TTY` is set), `gh … --web` would open a browser on the remote machine. So `o` copies the page URL to the clipboard of the local terminal instead. dispatch gets the URL with `gh <kind> view <n> --json url --jq .url`. Inside tmux it runs `tmux set-buffer -w <url>`, and tmux sends OSC 52 to its client. Outside tmux, dispatch writes OSC 52 itself.

## Sub-features

- `open-issue` opens the selected issue page.
- `open-pr` opens the selected PR page.
- `open-session` opens the issue of the selected issue session (`#N`), or the PR of a PR session (`PR #P`).
- `open-toast` shows `Opening issue #N on GitHub…` or `Opening PR #P on GitHub…`. A success adds no other toast. A failure shows `Opening issue #N failed: <gh error>.` or `Opening PR #P failed: <gh error>.`.
- `open-empty` shows `Nothing to open.` and calls no gh when no row is selected, for example when a filter hides every row.
- `copy-over-ssh` shows `Copying the PR #P URL…` (or `Copying the issue #N URL…` on an issue or session row), then `Copied <url>.`. The toast shows the URL because a terminal can ignore OSC 52 and give no error. A failure of gh or of the copy shows `Copying the PR #P URL failed: <error>.`, for example `gh pr view printed no URL`.

## How to get to it (user POV)

- Select a row on any tab and press `o`. While the filter input has focus, `o` is typed into the filter.
- When a row is selected, the key bar shows `o to open on GitHub`, or `o to copy URL` over SSH. Use it to check which path dispatch took at launch. When a list is empty, the key bar hides `o`.

## Driving it with verify.sh

Preconditions:

- A run is up and doctor shows no `FAIL`. `GH_BROWSER` is `$RUN/bin/browser`, so no browser opens.

- **Issue.** Press `1`, select `#N`, press `o`. Run `$V keys "$RUN" 1`, move with `j`/`k` until the row starts with `❯ `, then `$V keys "$RUN" o` and `$V wait "$RUN" 'Opening issue #N on GitHub'`. `$RUN/evidence/browser.log` gets `https://github.com/<owner>/<repo>/issues/N`. `gh-calls.log` has `gh issue view N --web`.
- **PR.** Press `2`, select `#P`, press `o`. The toast is `Opening PR #P on GitHub…`. `browser.log` gets `…/pull/P`, and `gh-calls.log` has `gh pr view P --web`.
- **Session.** A session for `#N` must exist ([Start a session](./start-session.md)). Press `3`, select the `#N` session, press `o`. The toast is `Opening issue #N on GitHub…`. `browser.log` gets `…/issues/N`, and `gh-calls.log` has `gh issue view N --web`.
- **PR session.** A PR session `PR #P` must exist (**PR session** in [Start a session](./start-session.md)). Press `3`, select the `PR #P` row, press `o`. The toast is `Opening PR #P on GitHub…`. `browser.log` gets `…/pull/P`, and `gh-calls.log` has `gh pr view P --web`.
- **Nothing selected.** Press `/`, type text that matches nothing, and press Enter. The key bar hides `o to open on GitHub` (and `o to copy URL`). Press `o`: the message row shows `Nothing to open.` and `gh-calls.log` gets no `--web` line. Press Esc to clear the filter.
- **Proof.** Run `$V shot "$RUN" open-toast` after each press, and quote the `browser.log` lines.
- **Copy over SSH.** Start the run with `$V up --over-ssh`. The key bar shows `o to copy URL`. Press `2`, select `#P`, press `o`, `$V wait "$RUN" 'Copying the PR #P URL'`, and `$V wait "$RUN" 'Copied https://'`. `$V tmux "$RUN" show-buffer` prints the URL. `gh-calls.log` has `gh pr view P --json url --jq .url`, and `browser.log` gets no line.
- **OSC 52 outside tmux.** Run `$V tmux "$RUN" set-option -g set-clipboard on`. Then start `env -u TMUX -u TMUX_PANE SSH_CONNECTION=… $BIN` in a new window of the run's server (the server's `TMUX_TMPDIR` keeps it off the user's tmux) and press `2 o` there. tmux turns dispatch's own OSC 52 into a buffer, and `show-buffer` prints the URL.

## Gotchas

- `browser.log` gets a line only when gh calls the browser. If the toast shows `Open … failed:`, the gh error is in the same toast.
- The `Opening …` toast is the only sign on the screen, and it goes after 5 s. Wait for it right after the key.
- `j` and `k` move from the current row. A row that an earlier step selected stays selected, so press `k` until the top row first if you need to go up.
- `up` removes `SSH_CONNECTION` and `SSH_TTY`, so a run from an SSH shell still tests the browser path. A plain `SSH_CONNECTION=… $V up` does not reach dispatch. Use `--over-ssh`.
- Outside this sandbox, `o` opens a real browser tab. Do not run `gh … --web` in the user's shell to check a URL.
