# dispatch verification map

This directory is the maintained source for verifying what a dispatch user sees and does. Read this index, then use the matching feature file as the recipe. The harness is `verify.sh` from [../SKILL.md](../SKILL.md).

## Baseline preconditions

- Build and launch with `V=.claude/skills/verify/scripts/verify.sh; $V build; $V up`. Keep the printed `RUN=` path.
- `$V doctor "$RUN"` shows no `FAIL`.
- The data is live GitHub data of the run's repo. It changes while you work: issues close, PRs merge. Pick targets from the screen at run time, never from numbers in this map.
- The journeys need at least one open issue, and the merge journey needs an open PR. If the default repo has none, `down` and `up --repo OWNER/NAME` with a repo that has them. Report which repo you used.
- Never drive a dispatch that this run did not start.

## Driving conventions

- Send keys only with `$V keys "$RUN" …` and `$V type "$RUN" …`. They go to the dispatch pane by its pane id, even when another window is active.
- After each key that changes state, `$V wait` for the text that proves the change. Do not sleep.
- Number keys switch tabs: `1` Issues, `2` Pull requests, `3` Sessions. The selected tab has `━` under it on screen line 2.
- Find a row by its `#<number> ` prefix. Move to it with `j` and `k`. The selected row starts with `▌`.

## Proof and skip reporting

- Save a `shot` before and after each action that you prove.
- Save a `state` after each action that changes git or tmux (start, stop, resume, merge handoff).
- Quote the log line that proves a side effect: `claude-calls.log` for a start or a sent prompt, `browser.log` for open, `gh-blocked.log` for a merge.
- Name the repo, the issue or PR number, and the evidence file for each claim.
- If a precondition is missing (no open issue, no open PR), report the journey as not verified and say why. Do not prove it through a different path.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph about the user-visible behavior. Then it has exactly these four H2 sections in this order:

1. `Sub-features`: short IDs with one line each.
2. `How to get to it (user POV)`: every user entry point.
3. `Driving it with verify.sh`: starts with `Preconditions:`, then labeled bullets that pair a user action, an exact command, and the result you can see.
4. `Gotchas`: traps that waste or invalidate a run.

## Features

- [Issue and PR lists](./lists.md): tabs, fetch freshness, refresh, filter, sort, PR icons on issue rows, pane resize.
- [Start a session](./start-session.md): the start dialog, its fields, and the worktree, marker, and window it makes.
- [Sessions](./sessions.md): session states, the output tail, attach, stop, and resume.
- [Merge](./merge.md): the two-press `m` flow, direct merge, and the handoff to an agent.
- [Open on GitHub](./open-on-github.md): `o` on an issue, a PR, or a session.
