# Open on GitHub

A user presses `o` to open the selected issue or PR in the browser. On the Sessions tab, `o` opens the session's issue. dispatch runs `gh issue view <n> --web` or `gh pr view <n> --web`, so it uses the host that `gh` is logged in to.

## Sub-features

- `open-issue` opens the selected issue page.
- `open-pr` opens the selected PR page.
- `open-session` opens the issue of the selected session.
- `open-toast` shows `Opening issue #N on GitHub` or `Opening PR #P on GitHub`.

## How to get to it (user POV)

- Select a row on any tab and press `o`.

## Driving it with verify.sh

Preconditions:

- A run is up and doctor shows no `FAIL`. `GH_BROWSER` is `$RUN/bin/browser`, so no browser opens.

- **Issue.** Press `1`, select `#N`, press `o`. Run `$V keys "$RUN" 1 o` and `$V wait "$RUN" 'Opening issue #N on GitHub'`. `$RUN/evidence/browser.log` gets `https://github.com/<owner>/<repo>/issues/N`. `gh-calls.log` has `gh issue view N --web`.
- **PR.** Press `2`, select `#P`, press `o`. `browser.log` gets `…/pull/P`, and `gh-calls.log` has `gh pr view P --web`.
- **Session.** Press `3`, select the `#N` session, press `o`. `browser.log` gets `…/issues/N`.
- **Proof.** Run `$V shot "$RUN" open-toast` after each press, and quote the `browser.log` lines.

## Gotchas

- `browser.log` gets a line only when gh calls the browser. If the toast shows `failed`, read the gh error in the message row.
- Outside this sandbox, `o` opens a real browser tab. Do not run `gh … --web` in the user's shell to check a URL.
