# dispatch: notes for agents

The checks hold most rules. This file holds only what the code cannot say.

## Before you commit

- Run `scripts/check.sh`. There is no CI, so this is the only gate.
- If you change what a user sees or does (keys, screens, toasts, start, attach, stop, merge), prove it in the real app with the `verify` skill. `cargo test` alone is not proof.
- If you change a key, a screen, or a flow, update its recipe in `.claude/skills/verify/features/` in the same commit. No check catches a stale recipe.
- Ask before you add a dependency.

## Issues and PRs

If an issue or PR is about something a user sees, put images of it in the body. Words alone are not sufficient. Add images for a PR that changes a screen, a key bar, a dialog, a toast, or a flow, and for an issue that reports a visible bug. A PR that changes only internals, tests, or docs needs no images.

- Show one state with a PNG. Show a flow (press a key, then see the result) with a GIF. GitHub cannot play an mp4 that `gh` uploads, so do not use video.
- Make the images from the `verify` run that proves the change. Do not make up a screen. Use `media.sh png` or `media.sh gif` on the `shot` files, then use `media.sh publish pr-<n>` (or `issue-<n>`) and paste the Markdown lines that it prints. The "Evidence" section of the `verify` skill gives the commands.
- For a PR, show the screen before the change and after it, when the change replaces something that a user already saw.

## Where a feature lives

A feature has one name. `src/app/<name>.rs` holds its state and flow, and `src/<name>/` or `src/session/` holds its rules and I/O. `src/ui/` is split by screen region (header, rows, footer, dialog, menus), not by feature. In `app/`, `ui/`, and `session/`, tests are in a `tests/` folder, one file per feature. Other modules keep an inline `mod tests`.

`app::update` changes `State`. The one other write is `main.rs`, which stores the hit map that the last draw returned. `ui` only reads. To do I/O from `app`, return a `Cmd`: `main.rs` runs `Attach`, `Copy`, and `Quit`, and `src/jobs.rs` runs each `Cmd::Job` and sends a `Msg` back.

## Rules that are not obvious

- User-visible sentences (refusals, errors, toasts) have tests that pin the exact text. A change to that text is a behavior change: make it a `fix` or `feat` commit, not a `refactor`.
- A toast is red because it is `toast::error`, never because of its words.
- `armed` (a pending second `m`) lives on `State`, not inside the list focus: a merge result can arm `m` while a menu is open.
- A PR head is a `PrBranch`. A head that is a bad branch name is `Unusable`: the PR still lists and merges, but dispatch never starts a fix session on it. A head in a fork is `Fork`: a fix session runs on the local branch `fork/pr-<n>`, only when the PR allows edits from maintainers. A fork's branch name never goes into a prompt, because the fork's author chose it.

## What stops each repeated mistake

| Mistake | What stops it |
|---|---|
| The start preview shows commands that differ from a real start | `session/start_argv.rs` builds argv for both |
| A file grows past 500 lines, or a `.rs` file has no `mod` line | `scripts/structure.sh` |
| An error crosses a module as a `String` | `scripts/structure.sh` |
| `app` or `ui` does I/O | `scripts/structure.sh` and clippy `disallowed-methods` |
| Two bools where one enum fits | clippy `max-struct-bools`, `max-fn-params-bools` |
| A silenced lint | `#[allow]` is denied; `#[expect]` needs a reason |
| The key bar cuts `? to keys` or `q to quit` | `ui/tests/footer.rs` |
| The verify sandbox leaks your shell environment | `verify.sh doctor` |
