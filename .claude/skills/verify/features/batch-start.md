# Start several sessions

A user marks several issues with Space, then starts a session for each with one key. One dialog lists the marked issues and the mode. Each session uses its default branch, prompt, and the chosen mode. A marked issue shows `◆` in place of its state glyph.

## Sub-features

- `batch-mark` toggles the mark of the selected issue with Space on the Issues tab. The key bar shows `space to mark` only when the selected issue has no session. An issue that has a session, or is starting, is not marked: the message row shows `#N already has a session`.
- `batch-open` opens the dialog with `s`, Enter, or a double-click while at least one issue is marked. The title is ` Start N sessions `, or ` Start 1 session ` for one issue. If no marked issue can start, no dialog opens, every mark clears, and the message row shows `No marked issue can start.`
- `batch-mode` shows the `Mode` radios. Left, Right, Space, Tab, and BackTab switch the mode. A click on an option picks it.
- `batch-start` starts one session per issue on Ctrl-S, Enter, or `[ Start ]`. The marks clear, dispatch goes to the Sessions tab, and the message row shows `Starting #A, #B`.
- `batch-cancel` closes the dialog with Esc or `[ Cancel ]` and keeps the marks.

## How to get to it (user POV)

- On the Issues tab, press Space on two or more issues, then press `s` or Enter.
- Double-click an issue row while issues are marked.

## Driving it with verify.sh

Preconditions:

- A run is up and `$V doctor "$RUN"` shows no `FAIL`.
- The Issues tab lists at least two open issues that have no session. Pick them at run time: `#A` and `#B`.

- **Mark.** Press `1`, move to `#A` with `j`/`k`, then run `$V keys "$RUN" Space`. Move to `#B` and press Space again. Run `$V screen "$RUN"`. Both rows start with `◆` in place of the state glyph, and the key bar has `space to mark`. Run `$V shot "$RUN" batch-marked`. Space on `#A` again removes its mark.
- **Open the dialog.** Run `$V keys "$RUN" s` and `$V wait "$RUN" 'Start 2 sessions'`. The dialog shows `Issues` with a line `#A <title>` and a line `#B <title>`, each title with Markdown rendered as in the rows, `Mode` with `● Implement directly`, the line `Each session uses its default branch and prompt.`, and `[ Start ]  [ Cancel ]`. The key bar is `ctrl+s to start · space to change mode · esc to cancel`. Run `$V shot "$RUN" batch-dialog`.
- **Cancel keeps the marks.** Run `$V keys "$RUN" Escape`. The dialog closes and both rows still show `◆`.
- **Mode.** Reopen with `s`, then run `$V keys "$RUN" Left`. The radio shows `● Plan first, then wait for approval`.
- **Start.** Run `$V keys "$RUN" C-s`, then `$V wait "$RUN" 'Starting #|Started #'`: the first `Started #N` toast can replace `Starting #A, #B` before the wait sees it. dispatch goes to the Sessions tab and the message row shows `Starting #A, #B` (numbers ascending). Run `$V wait "$RUN" '▲ 2 needs you' 20`. Run `$V shot "$RUN" batch-started`.
- **Side effects.** Run `$V state "$RUN" after-batch`. It shows one worktree, one `branch.<branch>.dispatch-issue` marker, and one window `#N` per issue. `$RUN/evidence/claude-calls.log` has one argv line per issue, each with `--permission-mode plan` if you chose plan first.
- **Marks gone.** Press `1`. No row starts with `◆`.
- **Refusal.** Select an issue that has a session and press Space. The message row shows `#N already has a session`, and the row stays unmarked.

## Gotchas

- Space only acts on the Issues tab. On the other tabs it does nothing.
- With marks, `s` never opens the single start dialog. Clear the marks (Space on each marked issue) to edit one issue's branch or prompt.
- The mode applies to every session of the batch.
- A start that fails shows `Start #N failed: <error>`. Mark and start the issue again: starts are idempotent.
- `Space` is the `tmux send-keys` name of the key. Do not use `' '`.
