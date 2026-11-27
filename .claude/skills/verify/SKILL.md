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
| checkout | `$RUN/work/<repo>`, a fresh shallow `gh repo clone` | Session worktrees go to `$RUN/work/<repo>/.claude/worktrees/<issue>`, never into the user's checkout. `up` sets `gh repo set-default <repo>`, so a fork does not read its parent. It also tracks every `origin` branch, as a normal clone does |
| Claude config | `$RUN/claude` (`CLAUDE_CONFIG_DIR`) | dispatch reads session transcripts from `projects/` here, never from your `~/.claude`. The fake claude writes one on each fresh start |
| config | `$RUN/xdg/dispatch/config.json` | `command` is `$RUN/bin/fake-claude` and `multiplexer` is `tmux`. Other entries of `~/.config` are symlinked, so `gh` and `git` keep their login and settings |
| `claude` | `$RUN/bin/fake-claude` | Logs its argv, runs dispatch's real `--settings` hooks (`SessionStart` on a fresh start, `Stop` sets `needs you`), echoes typed lines. Type `exit` to make it exit. `touch $RUN/ask-trust` makes a fresh start ask the trust question first, before any hook; type a line to answer |
| `gh` | `$RUN/bin/gh` (first on PATH) | Passes `issue list/view`, `pr list/view`, `repo view`, `auth status`, and `api graphql` queries (not mutations) to the real gh. Blocks every other call, `pr merge` too, and logs it |
| browser | `$RUN/bin/browser` (`GH_BROWSER`, `BROWSER`) | `o` goes through the real `gh … --web`. The URL goes to a log. No browser opens |
| environment | `up` and `restart` pass only the `KEEP_ENV` allowlist in `verify.sh` (`HOME`, `PATH`, `TERM`, locale, gh tokens, and a few more) | Any other variable can change what dispatch does. `HERDR_*` and `HC_*` make it start sessions outside the sandbox, and `SSH_CONNECTION` makes `o` copy a URL. `--over-ssh` sets a fake `SSH_CONNECTION` to test the copy path |

The `bin/` scripts are verification scaffolding. `down` deletes them.

Two runs can go at the same time. Each run has its own socket and dirs. Never drive a dispatch that you did not start with `up`, and never send keys to the user's tmux server.

## Launch

```sh
$V build                       # cargo build; prints target/debug/dispatch
$V up                          # prints RUN=/tmp/dispatch-verify/<id>
$V up --repo OWNER/NAME        # drive a different GitHub repo (read-only)
```

Options: `--size COLSxROWS` (default `160x45`), `--allow-merge` (lets `gh pr merge` through. Use it only on a sandbox repo that you own), `--over-ssh` (dispatch sees an SSH session, so `o` copies the URL).

`up` copies the binary into the run, clones the repo, starts dispatch in tmux session `drive`, window `dispatch`, and waits up to 30 s for the first fetch. Ready means the header shows `<repo> · updated Ns ago`. If the first fetch fails, `up` saves the screen to `evidence/` and exits non-zero. Run `down` then.

The default repo is the `origin` of this checkout. The journeys need live GitHub data (an open issue, an open PR). If the default repo has none, use `--repo` with a public repo that has some. The guard keeps it read-only.

Pick a repo with fewer than about 50 PRs in total (open and closed). dispatch asks for 100 PRs with their checks and merge state in one GraphQL call, and on a larger repo GitHub stops it after about 10 s with `HTTP 502` or `HTTP 504`. The PR tab then stays `No pull requests.` with `refresh failed`. This happens on `ratatui/ratatui`, for example. Repos that worked on 2026-11-27:

- `serkanyersen/dotstate` (41 PRs): open issues, an open `CLEAN` PR (direct merge, blocked by the guard), open PRs with failing checks that close no issue (PR-keyed new session and handoff), and fork PRs that allow maintainer edits and close an issue (fork new session).
- `viur-framework/vi-shoelace` worked on 2026-11-14, but on 2026-11-27 it had no open PR.

This checkout's own `origin` had no open PR on 2026-11-27, so pass `--repo`.

Data changes. Check a repo before you use it, with the command in [features/README.md](features/README.md).

Teardown is `$V down "$RUN"`.

## Doctor

Run this first, and again when anything looks wrong. It is read-only.

```sh
$V doctor "$RUN"
```

It checks: the run's tmux server answers, the drive pane is alive and runs this run's binary, the environment of the drive pane is readable, the drive pane has the gh guard first on `PATH`, `TMUX` in the drive pane points at the run's socket, the drive pane has no herdr, herdr app, or SSH variables (except `SSH_CONNECTION` with `--over-ssh`) and no variable outside the allowlist, the binary is the same as `target/debug/dispatch` and is not older than `src/`, `gh` is logged in as the drive server sees it, the clone has `origin/HEAD` and tracks every `origin` branch, `gh` in the clone resolves to the run's repo, the last fetch did not fail, and the header shows a fetch age. Doctor reads the pane process from `/proc` on Linux and from `ps` on macOS. If it cannot read the environment, the two leak checks fail too: they never pass on an empty answer. It then prints the tab row, the repo and freshness line, and the key bar. A `FAIL` on the binary checks means you changed the code after `up`: run `build`, `down`, `up`. The fetch-age check fails on the Sessions tab, because only tabs 1 and 2 show an age. Press `1` and run it again.

## Drive

```sh
$V keys "$RUN" 1 j s           # tmux key names: j k s a x m o r S / q Enter Escape Tab BTab C-s Up Down
$V type "$RUN" 'text'          # literal text, for the filter or a dialog field
$V wait "$RUN" 'needs you' 20  # wait until the screen matches a grep -E regex; on timeout prints the screen, exits 1
$V screen "$RUN"               # print the screen now
$V window "$RUN" '#26'         # print the pane of a session window by name ('#26', 'PR #12')
$V client "$RUN"               # attach a headless tmux client; needed before testing `a`
$V tmux "$RUN" ARGS...         # any tmux command against this run's server only
$V restart "$RUN"              # start dispatch again in the drive pane, as a user who quits and reruns it
```

To start dispatch again, use `restart`, never `$V tmux "$RUN" respawn-pane …`. tmux gives the new pane the `PATH` of the tmux client that sends the command, so a plain respawn starts dispatch with the real `gh` and no guard. `restart` starts dispatch with only the environment `up` gave the tmux server (the `KEEP_ENV` allowlist in `verify.sh`). Doctor fails on `gh guard first on PATH` if the guard is lost.

Stable handles on the screen (line numbers are `capture-pane` lines, from 1):

- Line 1 is blank. Line 2 is the tab row: ` 1  Issues N      2  Pull requests N      3  Sessions N`. The counts have no parentheses. The selected tab has a colored badge, not an underline, so read it from the `.ansi` shot or from the key bar.
- Lines 4 to 6 are the header next to the logo: `dispatch vX.Y.Z`, then `<repo> · updated Ns ago` (or `loading…`, `refreshing…`, `refresh failed Ns ago`; tabs 1 and 2 only), then `▲ N needs you   ● N working   ○ N exited   ✓ N merged`. This line is always on screen, so `wait 'needs you'` matches at once. Wait for `▲ 1 needs you`.
- Rows are in groups with a heading and a count, for example `Ready for review 3`. A row is `❯ <glyph> #<number> <title>  <summary>  <age>`. The selected row starts with `❯`, other rows with two spaces. Find a row with `grep '#N '`.
- The message row, above the prompt rules, shows toasts, for example `Stopped #26.`, `Opening issue #26 on GitHub…`, `Press m again to merge PR #12.`. An info toast stays for 5 s. An error toast (red) stays until the next key. A long toast wraps onto as many as 4 rows above the prompt, and a longer one ends in `…`. A wrapped toast breaks at spaces, so `wait` for a short part of it, not the whole sentence.
- The prompt line, between two rules, shows a hint such as `❯ / to filter issues · s to start a session`, or the kept filter `❯ /word`.
- The key bar (last line) shows `<key> to <action>` pairs joined by ` · `, only for keys that work on the selected row, for example `? to show keys · s to start session · m to merge · o to open on GitHub · q to quit`. Keys are written as the keys menu writes them (`ctrl+s`, `down/up`). `S sort: <order>` is at its right end. A dialog or menu has its own key bar, and then the prompt hint and `S sort: <order>` do not show.
- The start dialog title is `Start session for #<n> <title>`. Its `Runs` block shows the exact commands.
- Session state glyphs: `●` running, `▲` needs you, `○` exited, and a purple `✓` when the PR on the session's branch is merged (unicode icon set; the `icons` setting can change them to Nerd Font glyphs). A row shows only the glyph. To wait for a state, match the header count (`● 1 working`, `○ 1 exited`) or the group heading (`Working 1`, `Exited 1`).

Wait for text, never sleep for a fixed time. dispatch draws every 100 ms, and gh calls take 1 to 3 s.

## Evidence

```sh
$V shot "$RUN" start-dialog    # evidence/NN-start-dialog.txt and .ansi (with colors)
$V state "$RUN" after-start    # evidence/NN-after-start.state.txt: git worktree list, dispatch markers, tmux windows
```

To put shots in an issue or PR (AGENTS.md says when), make images from the `.ansi` files and publish them:

```sh
M=.claude/skills/verify/scripts/media.sh
$M png "$RUN"/evidence/03-after.ansi                       # evidence/03-after.png
$M gif "$RUN"/evidence/flow.gif "$RUN"/evidence/0[1-3]-*.ansi  # one GIF frame per shot, in order
$M publish pr-117 "$RUN"/evidence/03-after.png "$RUN"/evidence/flow.gif
```

`publish` pushes the files to the `media` branch (it has no code, and nothing merges it) and prints one `![name](url)` line for each file. Paste these lines into the body. Each URL contains the commit, so an image does not change after you publish it. Run `publish` before `down` or after it: `down` keeps `evidence/`. Do the steps in this order: make the shots, make the images, publish them, then use `gh pr create` or `gh issue create`. `media.sh` needs `aha`, `chromium`, and `magick`.

The run also writes these logs to `$RUN/evidence/` by itself:

- `gh-calls.log`: each gh call dispatch made.
- `gh-blocked.log`: each gh call that the guard blocked.
- `claude-calls.log`: each fake-claude start with cwd, pane, and full argv, each hook it ran, and each line it got. The argv is quoted with `printf %q`, so a space in the prompt is `\ `. Search for `gh\ pr\ checkout\ 17`, not `gh pr checkout 17`.
- `browser.log`: each URL that `o` opened.

Proof standards:

- Drive the real user path: keys in the dispatch pane. Do not call `git worktree add` or `tmux new-window` yourself to make a state.
- Capture the action and the result: a `shot` before the key and after it.
- Check side effects with `state` and the logs, not only the screen. A start is proven by the worktree, the `branch.<b>.dispatch-issue` (or `dispatch-pr`) and `dispatch-harness` markers, the tagged window, and the argv in `claude-calls.log`.
- The fake claude is the one mock. It stands at the `command` boundary that the config already exposes. `codex`, `opencode`, `agy`, and `pi` stand-ins (`fake-agent`) sit on `PATH` for the other harnesses. The hooks it runs are dispatch's real hooks.
- The gh guard does not fake answers. It passes reads to GitHub and refuses writes. A blocked merge proves the exact argv dispatch would run (in `gh-blocked.log`). It does not prove that GitHub accepts the merge.

## Cleanup

```sh
$V down "$RUN"
$V list                        # every run under the root, UP or down
```

`down` kills only this run's tmux server (by its socket) and the headless client's process group, then deletes `work/`, `xdg/`, `bin/`, and `tmux/`. It keeps `$RUN/evidence/` and `$RUN/run.env`. It never kills by process name. Run `down` after every failed attempt too. Delete old evidence with `rm -rf /tmp/dispatch-verify/<id>` when you no longer need it.

## Feature map

Recipes per feature are in [features/README.md](features/README.md). Read the index, then the feature file, before you drive.
