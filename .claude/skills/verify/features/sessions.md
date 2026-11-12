# Sessions

The Sessions tab lists each session that dispatch started, with its state, branch, worktree, linked PR, and the last lines of the Claude pane. From any tab a user can attach to a session, stop it, or resume an exited one. dispatch keeps no state file: it reads sessions from git and tmux each time.

## Sub-features

- `sessions-state` shows `● running`, `▲ needs you`, or `○ exited`, and `▲N` on the tab row.
- `sessions-tail` shows the last lines of the Claude pane under `output ─`.
- `sessions-attach` moves the tmux client to the session window with `a` on any tab, or Enter on the Sessions tab.
- `sessions-stop` kills the window with `x` on a live session. The worktree and the branch stay. On an exited session, `x` shows `No running session here.`
- `sessions-resume` runs `claude --continue` in the worktree with `a` (or Enter on the Sessions tab) on an exited session. The key bar shows `a resume`. A missing window (after `x`) gets a new `#N` window. A dead pane (after the agent exits) is respawned in the same window.
- `sessions-derived` lists the same sessions after dispatch restarts.

## How to get to it (user POV)

- Press `3` for the Sessions tab.
- On the Issues tab, a row with a session shows its state at the right end. `a` and `x` work there too.
- Press `a` on a session to attach, or Enter on the Sessions tab. Press `x` to stop. On the Issues tab, Enter on an issue with a session only goes to the Sessions tab.

## Driving it with verify.sh

Preconditions:

- A run is up and doctor shows no `FAIL`.
- A session for issue `#N` exists. Make it with [Start a session](./start-session.md).

- **State and tail.** Press `3`. Run `$V keys "$RUN" 3` and `$V wait "$RUN" 'FAKE-CLAUDE waiting'`. The detail pane shows `state    ▲ needs you`, `branch`, `worktree …/.claude/worktrees/N`, and under `output ─` the fake claude lines. The worktree path is cut from the left with `…` when it is too wide, so match `worktree .*\.claude/worktrees/N`. The key bar shows `enter attach   a attach   x stop`. Run `$V shot "$RUN" sessions-tab`.
- **Input.** Type a line into the session window, as a user does after attach. Run `$V tmux "$RUN" send-keys -t '=drive:#N' 'hello' Enter`, `$V wait "$RUN" 'FAKE-CLAUDE got: hello'`, and `$V wait "$RUN" 'state +▲ needs you'`. `claude-calls.log` gets `hook UserPromptSubmit`, `got: hello`, and `hook Stop`. Do not wait for `● running` here: the fake claude is busy for 1 s, and dispatch reads the state every 2 s, so it often never shows. See **Resume** for a reliable `● running`.
- **Attach.** Attach a client, then press `a`. Run `$V client "$RUN"`, `$V tmux "$RUN" select-window -t drive:dispatch`, `$V keys "$RUN" a`, and `$V tmux "$RUN" display -p -t drive '#{window_name}'`. It prints `#N`. Go back with `$V tmux "$RUN" select-window -t drive:dispatch`.
- **Stop.** Press `x`. Run `$V keys "$RUN" x` and `$V wait "$RUN" 'Stopped #N'` (`Stopping #N` comes first). The row shows `○ exited` and the key bar shows `a resume`. `$V state "$RUN" after-stop` shows the worktree and the marker, and no `#N` window.
- **Resume.** Press `a` on the exited session. Run `$V keys "$RUN" a` and `$V wait "$RUN" 'Resumed #N\. Press a to attach\.'` (`Resuming #N with claude --continue` comes first). Then `$V wait "$RUN" '● running' 3`: dispatch reads the state at once after a resume, and the fake claude is busy for 2 s at start. Then the row shows `▲ needs you`. `claude-calls.log` has a new line whose argv starts with `--continue --settings`. `$V state` shows a new `#N` window id with the worktree tag.
- **Exit.** Type `exit` into the session window. Run `$V tmux "$RUN" send-keys -t '=drive:#N' exit Enter`. The row shows `○ exited`. `$V state` shows the `#N` window with `dead=1` (remain-on-exit).
- **Resume a dead pane.** Press Enter on the exited session (Sessions tab). Run `$V keys "$RUN" Enter` and `$V wait "$RUN" 'Resumed #N'`. `$V state` shows the same window id as after **Exit**, now with `dead=0`.
- **Restart.** Quit and start dispatch again in the same run. Run `$V keys "$RUN" q`, then `$V restart "$RUN"` (it waits for `updated`). Run `$V doctor "$RUN"`. Press `3`. The `#N` session is still listed with the same state.

## Gotchas

- `a` without an attached client fails with `attach failed: tmux failed with code 1: no current client`. dispatch runs `select-window ; switch-client`, and the switch needs a client. Run `$V client "$RUN"` first.
- After `a`, the active tmux window is `#N`. `$V keys` and `$V screen` still reach dispatch by pane id. Select `drive:dispatch` again before you check what a user sees.
- `x` and `a` act on the selected row. On the Issues tab, select the issue row first.
- The `needs you` state needs `detect_waiting` (on in the run config). If the window tag shows `waiting=` empty after the fake claude prints `waiting for input`, the hooks did not run: read `claude-calls.log`.
- `restart` in **Restart** uses tmux, not a user key. It stands in for the user who starts dispatch again. Do not use `$V tmux "$RUN" respawn-pane` for this: tmux gives the pane the `PATH` of your shell, so dispatch runs the real `gh` with no guard. Doctor then fails on `gh guard first on PATH`.
