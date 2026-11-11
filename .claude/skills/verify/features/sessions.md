# Sessions

The Sessions tab lists each session that dispatch started, with its state, branch, worktree, linked PR, and the last lines of the Claude pane. From any tab a user can attach to a session, stop it, or resume an exited one. dispatch keeps no state file: it reads sessions from git and tmux each time.

## Sub-features

- `sessions-state` shows `● running`, `▲ needs you`, or `○ exited`, and `▲N` on the tab row.
- `sessions-tail` shows the last lines of the Claude pane under `output ─`.
- `sessions-attach` moves the tmux client to the session window with `a` or Enter.
- `sessions-stop` kills the window with `x`. The worktree and the branch stay.
- `sessions-resume` runs `claude --continue` in the worktree with `a` on an exited session.
- `sessions-derived` lists the same sessions after dispatch restarts.

## How to get to it (user POV)

- Press `3` for the Sessions tab.
- On the Issues tab, a row with a session shows its state at the right end. `a` and `x` work there too.
- Press `a` or Enter on a session to attach. Press `x` to stop.

## Driving it with verify.sh

Preconditions:

- A run is up and doctor shows no `FAIL`.
- A session for issue `#N` exists. Make it with [Start a session](./start-session.md).

- **State and tail.** Press `3`. Run `$V keys "$RUN" 3` and `$V wait "$RUN" 'FAKE-CLAUDE waiting'`. The detail pane shows `state    ▲ needs you`, `branch`, `worktree $RUN/work/<repo>-N`, and under `output ─` the fake claude lines. Run `$V shot "$RUN" sessions-tab`.
- **Running.** Type a line into the session window, as a user does after attach. Run `$V tmux "$RUN" send-keys -t '=drive:#N' 'hello' Enter` and `$V wait "$RUN" '● running' 5`. The fake claude runs the `UserPromptSubmit` hook, so the row shows `● running` for 1 s, then `▲ needs you` again. The tail shows `FAKE-CLAUDE got: hello`.
- **Attach.** Attach a client, then press `a`. Run `$V client "$RUN"`, `$V tmux "$RUN" select-window -t drive:dispatch`, `$V keys "$RUN" a`, and `$V tmux "$RUN" display -p -t drive '#{window_name}'`. It prints `#N`. Go back with `$V tmux "$RUN" select-window -t drive:dispatch`.
- **Stop.** Press `x`. Run `$V keys "$RUN" x` and `$V wait "$RUN" 'Stopped #N'`. The row shows `○ exited`. `$V state "$RUN" after-stop` shows the worktree and the marker, and no `#N` window.
- **Resume.** Press `a` on the exited session. Run `$V keys "$RUN" a` and `$V wait "$RUN" 'Resumed #N'`. The row shows `▲ needs you` after 2 s. `claude-calls.log` has a new line whose argv starts with `--continue --settings`. `$V state` shows a new `#N` window with the worktree tag.
- **Exit.** Type `exit` into the session window. Run `$V tmux "$RUN" send-keys -t '=drive:#N' exit Enter`. The row shows `○ exited`. `$V state` shows the `#N` window with `dead=1` (remain-on-exit).
- **Restart.** Quit and start dispatch again in the same run. Run `$V keys "$RUN" q`, then `$V tmux "$RUN" respawn-pane -k -t drive:dispatch "$RUN/bin/dispatch"` and `$V wait "$RUN" 'updated'`. Press `3`. The `#N` session is still listed with the same state.

## Gotchas

- `a` without an attached client fails with `attach failed: tmux failed with code 1: no current client`. dispatch runs `select-window ; switch-client`, and the switch needs a client. Run `$V client "$RUN"` first.
- After `a`, the active tmux window is `#N`. `$V keys` and `$V screen` still reach dispatch by pane id. Select `drive:dispatch` again before you check what a user sees.
- `x` and `a` act on the selected row. On the Issues tab, select the issue row first.
- The `needs you` state needs `detect_waiting` (on in the run config). If the window tag shows `waiting=` empty after the fake claude prints `waiting for input`, the hooks did not run: read `claude-calls.log`.
- The respawn in **Restart** uses tmux, not a user key. It stands in for the user who starts dispatch again. The env of the server is the same as at `up`.
