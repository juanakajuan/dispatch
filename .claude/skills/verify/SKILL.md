---
name: verify
description: Launch and drive the dispatch terminal UI (Rust, ratatui) the way a user does, in a private tmux server with a fresh clone, a stand-in claude, and a gh guard that blocks GitHub writes. Use it to prove a change to dispatch works in the real app (tabs, start dialog, sessions, attach, stop, merge, open), not only in cargo test.
---

# Verify dispatch

dispatch is a TUI for one GitHub repo. It reads issues and PRs with `gh`, and it starts each Claude session in a git worktree and a tmux window. This skill runs it in a disposable sandbox and drives it with `tmux send-keys` and `capture-pane`.

All commands use the helper `.claude/skills/verify/scripts/verify.sh` (below: `V`). Run it from any directory in this checkout. It builds and runs the code of the checkout it lives in.

```sh
V=.claude/skills/verify/scripts/verify.sh
```

## What the sandbox isolates

Each run is a directory `RUN=/tmp/dispatch-verify/<id>` (set `DISPATCH_VERIFY_HOME` to move the root):

| Part | Where | Why |
|---|---|---|
| tmux server | socket `$RUN/tmux/sock`, no user tmux.conf | The user's tmux sessions are never touched. dispatch runs inside this server, so its windows go there too |
| checkout | `$RUN/work/<repo>`, a fresh shallow `gh repo clone` | Session worktrees go to `$RUN/work/<repo>/.claude/worktrees/<issue>`, never into the user's checkout |
| config | `$RUN/xdg/dispatch/config.json` | `claude_command` is `$RUN/bin/fake-claude`. Other entries of `~/.config` are symlinked, so `gh` and `git` keep their login and settings |
| `claude` | `$RUN/bin/fake-claude` | Logs its argv, runs dispatch's real `--settings` hooks (`Stop` sets `needs you`), echoes typed lines. Type `exit` to make it exit |
| `gh` | `$RUN/bin/gh` (first on PATH) | Passes `issue list/view`, `pr list/view`, `repo view`, `auth status` to the real gh. Blocks every other call, `pr merge` too, and logs it |
| browser | `$RUN/bin/browser` (`GH_BROWSER`, `BROWSER`) | `o` goes through the real `gh … --web`. The URL goes to a log. No browser opens |

The `bin/` scripts are verification scaffolding. `down` deletes them.

Two runs can go at the same time. Each run has its own socket and dirs. Never drive a dispatch that you did not start with `up`, and never send keys to the user's tmux server.

## Launch

```sh
$V build                       # cargo build; prints target/debug/dispatch
$V up                          # prints RUN=/tmp/dispatch-verify/<id>
$V up --repo OWNER/NAME        # drive a different GitHub repo (read-only)
```

Options: `--size COLSxROWS` (default `160x45`), `--allow-merge` (lets `gh pr merge` through. Use it only on a sandbox repo that you own).

`up` copies the binary into the run, clones the repo, starts dispatch in tmux session `drive`, window `dispatch`, and waits up to 30 s for the first fetch. Ready means the tab row shows `updated Ns ago`. If the first fetch fails, `up` saves the screen to `evidence/` and exits non-zero. Run `down` then.

The default repo is the `origin` of this checkout. The journeys need live GitHub data (an open issue, an open PR). If the default repo has none, use `--repo` with a public repo that has some, for example `--repo ratatui/ratatui`. The guard keeps it read-only.

Teardown is `$V down "$RUN"`.

## Doctor

Run this first, and again when anything looks wrong. It is read-only.

```sh
$V doctor "$RUN"
```

It checks: the run's tmux server answers, the drive pane is alive and runs this run's binary, the binary is the same as `target/debug/dispatch` and is not older than `src/`, `gh` is logged in as the drive server sees it, the clone has `origin/HEAD`, the last fetch did not fail, and the tab row shows a fetch age. It then prints the tab row and the message row. A `FAIL` on the binary checks means you changed the code after `up`: run `build`, `down`, `up`. The fetch-age check fails on the Sessions tab, because only tabs 1 and 2 show an age. Press `1` and run it again.

## Drive

```sh
$V keys "$RUN" 1 j s           # tmux key names: j k s a x m o r S / q Enter Escape Tab BTab C-s Up Down
$V type "$RUN" 'text'          # literal text, for the filter or a dialog field
$V wait "$RUN" 'needs you' 20  # wait until the screen matches a grep -E regex; on timeout prints the screen, exits 1
$V screen "$RUN"               # print the screen now
$V window "$RUN" '#26'         # print the pane of a session window (the fake claude output)
$V client "$RUN"               # attach a headless tmux client; needed before testing `a`
$V tmux "$RUN" ARGS...         # any tmux command against this run's server only
```

Stable handles on the screen:

- Tab row (line 1): `1 Issues N`, `2 Pull requests N`, `3 Sessions N`, then `▲N` for sessions that need you, `S sort: <order>`, and on tabs 1 and 2 `updated Ns ago`, `refreshing…`, or `refresh failed Ns ago`. The selected tab has `━` under it on line 2.
- List rows start with `#<number> ` and the selected row starts with `▌`.
- The message row (second last line) shows toasts, for example `Stopped #26`, `Opening issue #26 on GitHub`, `Press m again to merge PR #12`.
- The key bar (last line) changes by tab. Use it to confirm which tab has focus.
- The start dialog title is `Start session for #<n> <title>`. Its `Runs` block shows the exact commands.
- Session states: `● running`, `▲ needs you`, `○ exited` (unicode icon set).

Wait for text, never sleep for a fixed time. dispatch draws every 100 ms, and gh calls take 1 to 3 s.

## Evidence

```sh
$V shot "$RUN" start-dialog    # evidence/NN-start-dialog.txt and .ansi (with colors)
$V state "$RUN" after-start    # evidence/NN-after-start.state.txt: git worktree list, dispatch markers, tmux windows
```

The run also writes these logs to `$RUN/evidence/` by itself:

- `gh-calls.log`: each gh call dispatch made.
- `gh-blocked.log`: each gh call that the guard blocked.
- `claude-calls.log`: each fake-claude start with cwd, pane, and full argv, each hook it ran, and each line it got.
- `browser.log`: each URL that `o` opened.

Proof standards:

- Drive the real user path: keys in the dispatch pane. Do not call `git worktree add` or `tmux new-window` yourself to make a state.
- Capture the action and the result: a `shot` before the key and after it.
- Check side effects with `state` and the logs, not only the screen. A start is proven by the worktree, the `branch.<b>.dispatch-issue` marker, the tagged window, and the argv in `claude-calls.log`.
- The fake claude is the one mock. It stands at the `claude_command` boundary that the config already exposes. The hooks it runs are dispatch's real hooks.
- The gh guard does not fake answers. It passes reads to GitHub and refuses writes. A blocked merge proves the exact argv dispatch would run (in `gh-blocked.log`). It does not prove that GitHub accepts the merge.

## Cleanup

```sh
$V down "$RUN"
$V list                        # every run under the root, UP or down
```

`down` kills only this run's tmux server (by its socket) and the headless client's process group, then deletes `work/`, `xdg/`, `bin/`, and `tmux/`. It keeps `$RUN/evidence/` and `$RUN/run.env`. It never kills by process name. Run `down` after every failed attempt too. Delete old evidence with `rm -rf /tmp/dispatch-verify/<id>` when you no longer need it.

## Feature map

Recipes per feature are in [features/README.md](features/README.md). Read the index, then the feature file, before you drive.
