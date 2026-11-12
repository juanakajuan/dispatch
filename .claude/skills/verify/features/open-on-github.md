# Open on GitHub

A user presses `o` to open the selected issue or PR in the browser. On the Sessions tab, `o` opens the session's issue. dispatch runs `gh issue view <n> --web` or `gh pr view <n> --web`, so it uses the host that `gh` is logged in to.

## Sub-features

- `open-issue` opens the selected issue page.
- `open-pr` opens the selected PR page.
- `open-session` opens the issue of the selected session.
- `open-toast` shows `Opening issue #N on GitHub` or `Opening PR #P on GitHub`. A success adds no other toast. A failure shows `Open issue #N failed: <gh error>` or `Open PR #P failed: <gh error>`.
- `open-empty` shows `Nothing to open.` and calls no gh when no row is selected, for example when a filter hides every row.

## How to get to it (user POV)

- Select a row on any tab and press `o`. While the filter input has focus, `o` is typed into the filter.

## Driving it with verify.sh

Preconditions:

- A run is up and doctor shows no `FAIL`. `GH_BROWSER` is `$RUN/bin/browser`, so no browser opens.

- **Issue.** Press `1`, select `#N`, press `o`. Run `$V keys "$RUN" 1`, move with `j`/`k` until the row starts with `▌ #N `, then `$V keys "$RUN" o` and `$V wait "$RUN" 'Opening issue #N on GitHub'`. `$RUN/evidence/browser.log` gets `https://github.com/<owner>/<repo>/issues/N`. `gh-calls.log` has `gh issue view N --web`.
- **PR.** Press `2`, select `#P`, press `o`. The toast is `Opening PR #P on GitHub`. `browser.log` gets `…/pull/P`, and `gh-calls.log` has `gh pr view P --web`.
- **Session.** A session for `#N` must exist ([Start a session](./start-session.md)). Press `3`, select the `#N` session, press `o`. The toast is `Opening issue #N on GitHub`. `browser.log` gets `…/issues/N`, and `gh-calls.log` has `gh issue view N --web`.
- **Nothing selected.** Press `/`, type text that matches nothing, press Enter, then `o`. The message row shows `Nothing to open.` and `gh-calls.log` gets no `--web` line. Press Esc to clear the filter.
- **Proof.** Run `$V shot "$RUN" open-toast` after each press, and quote the `browser.log` lines.

## Gotchas

- `browser.log` gets a line only when gh calls the browser. If the toast shows `Open … failed:`, the gh error is in the same toast.
- The `Opening …` toast is the only sign on the screen, and it goes after 5 s. Wait for it right after the key.
- `j` and `k` move from the current row. A row that an earlier step selected stays selected, so press `k` until the top row first if you need to go up.
- Outside this sandbox, `o` opens a real browser tab. Do not run `gh … --web` in the user's shell to check a URL.
