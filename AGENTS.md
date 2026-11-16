# dispatch: notes for agents

The checks hold most rules. This file holds only what the code cannot say.

## Before you commit

- Run `scripts/check.sh`. CI runs the same script.
- If you change what a user sees or does (keys, screens, toasts, start, attach, stop, merge), prove it in the real app with the `verify` skill. `cargo test` alone is not proof.
- If you change a key, a screen, or a flow, update its recipe in `.claude/skills/verify/features/` in the same commit. No check catches a stale recipe.
- Ask before you add a dependency.

## Where a feature lives

A feature has one name. `src/app/<name>.rs` holds its state and flow, and `src/<name>/` or `src/session/` holds its rules and I/O. `src/ui/` is split by screen region (header, rows, footer, dialog, menus), not by feature. In `app/`, `ui/`, and `session/`, tests are in a `tests/` folder, one file per feature. Other modules keep an inline `mod tests`.

`app::update` changes `State`. The one other write is `main.rs`, which stores the hit map that the last draw returned. `ui` only reads. To do I/O from `app`, return a `Cmd`: `main.rs` runs `Attach`, `Copy`, and `Quit`, and `src/jobs.rs` runs each `Cmd::Job` and sends a `Msg` back.

## Rules that are not obvious

- User-visible sentences (refusals, errors, toasts) have tests that pin the exact text. A change to that text is a behavior change: make it a `fix` or `feat` commit, not a `refactor`.
- A toast is red because it is `toast::error`, never because of its words.
- `armed` (a pending second `m`) lives on `State`, not inside the list focus: a merge result can arm `m` while a menu is open.
- A PR head is a `PrBranch`. A head that is a bad branch name or is in a fork is `Unusable`: the PR still lists and merges, but dispatch never starts a fix session on it.

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
