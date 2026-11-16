# Start a session

A user selects an open issue and starts a Claude session for it. A dialog shows the branch, the mode, the prompt, and the exact commands. On Ctrl-S dispatch makes a git worktree on a new branch, marks the branch with the issue number, and opens a multiplexer window that runs `claude` in the worktree. The sandbox pins the multiplexer to tmux.

## Sub-features

- `start-open` opens the dialog from the Issues tab with `s`, Enter, or a double-click.
- `start-fields` shows a branch name `<prefix>/<n>-<slug>`, the mode (plan first or implement), and an editable prompt. The prefix comes from the first label that matches: `bug` gives `fix`, `docs` or `documentation` gives `docs`, `chore` gives `chore`, else `feat`. The slug is the first 4 words of the title.
- `start-preview` shows `git worktree add .claude/worktrees/<n> -b <branch> <base>` and the configured `claude_command` with the prompt under `Runs`. The preview has no `--settings`. With the herdr backend the first line is `herdr worktree create …` instead.
- `start-run` makes the worktree, the `branch.<b>.dispatch-issue` marker, and a tagged tmux window named `#<n>`.
- `start-trust` shows the session as `needs you` while Claude asks to trust the worktree, and `running` after you answer. dispatch sets the waiting flag on the new window, and Claude's `SessionStart` hook clears it.
- `start-plan-mode` adds `--permission-mode plan` to the argv when the mode is plan first.
- `start-idempotent` goes to the existing session instead of making a second worktree. While a start is still running, `s` shows `#N is already starting`.
- `start-cancel` closes the dialog with Esc and changes nothing.
- `start-mouse`: a click on a field label focuses it, a click on a mode option picks it, a click in a text input puts the cursor there, and the `[ Start ]` and `[ Cancel ]` buttons on the last dialog row act as Ctrl-S and Esc.

## How to get to it (user POV)

- On the Issues tab, select an issue and press `s`.
- On the Issues tab, select an issue and press Enter.
- Double-click an issue row.

## Driving it with verify.sh

Preconditions:

- A run is up and `$V doctor "$RUN"` shows no `FAIL`.
- The Issues tab lists at least one open issue `#N` that has no session.

- **Pick the issue.** Press `1`. Run `$V keys "$RUN" 1`, then `$V screen "$RUN"`, and pick the `#N` of a row. Move to it with `j`/`k`. The row starts with `❯`. Run `$V shot "$RUN" issues-before`.
- **Open the dialog.** Press `s`. Run `$V keys "$RUN" s` and `$V wait "$RUN" 'Start session for #N '`. The dialog shows `› Branch`, `Mode` with `● Implement directly`, `Prompt` starting `Implement GitHub issue #N:`, and under `Runs` the lines `$ git worktree add .claude/worktrees/N -b <branch> origin/<default>` and `$ $RUN/bin/fake-claude "Implement GitHub issue #N: …"`. The last dialog row is `[ Start ]  [ Cancel ]`. The key bar is `ctrl-s to start · tab to next field · esc to cancel`. Run `$V shot "$RUN" start-dialog`.
- **Cancel.** Press Esc. Run `$V keys "$RUN" Escape`. The dialog closes. `$V state "$RUN" after-cancel` shows only the main worktree and `(none)` under markers.
- **Plan mode (optional).** Reopen with `s`, press `Tab` to focus `Mode`, then `Left`. Run `$V keys "$RUN" s Tab Left`. The radio shows `● Plan first, then wait for approval`, the field shows `› Mode`, and the second `Runs` line has `--permission-mode plan`. Space also changes the mode.
- **Start.** Press Ctrl-S. Run `$V keys "$RUN" C-s`, `$V wait "$RUN" 'Starting #N|Started #N'`, and `$V wait "$RUN" '▲ 1 needs you' 20`. dispatch moves to the Sessions tab. The `#N` row shows `❯ ▲ #N` (the fake claude runs dispatch's `Stop` hook after 2 s), the tab row shows `3  Sessions 1`, and header line 6 shows `▲ 1 needs you   ● 0 working   ○ 0 exited`. Do not wait for `needs you` alone: the header always shows it, also as `▲ 0 needs you`. Run `$V shot "$RUN" after-start`. A failed start shows `Start #N failed: <error>` for 5 s.
- **Side effects.** Run `$V state "$RUN" after-start`. It shows the worktree `$RUN/work/<repo>/.claude/worktrees/N` on `[<branch>]`, the marker `branch.<branch>.dispatch-issue N`, and a window `#N` with `tag=$RUN/work/<repo>/.claude/worktrees/N waiting=1`. `grep -cx '.claude/worktrees/' $RUN/work/<repo>/.git/info/exclude` prints `1`. Do not use `git status` as proof of the exclude: a repo that has `.claude/worktrees/` in its `.gitignore` (dispatch does) hides the worktrees without it.
- **Claude argv.** Read `$RUN/evidence/claude-calls.log`. The first line has `cwd=$RUN/work/<repo>/.claude/worktrees/N` and an argv with `--settings` (the hooks JSON) and the prompt as one argument. With plan mode, the argv starts with `--permission-mode plan`. The next lines are `hook SessionStart`, `hook UserPromptSubmit`, and `hook Stop`.
- **Trust question.** Before a start, run `touch $RUN/ask-trust`. Start an issue `#N` as above. Run `$V wait "$RUN" '▲ 1 needs you' 20`, then `$V keys "$RUN" a` to attach. The pane shows `FAKE-CLAUDE Accessing workspace: do you trust this folder?`. `claude-calls.log` has no `hook` line after the argv line yet. Type `yes` and Enter, detach with `C-b d`. The row goes to `● running`, then `▲ needs you` after 2 s. Run `rm $RUN/ask-trust` after.
- **Start again.** Press `1`, select `#N`, press `s`. The UI goes to the existing session and no dialog opens. `$V state` still shows one worktree for `N`.

## Gotchas

- The live repo can have no open issue. Then this journey is not verified. Use `up --repo OWNER/NAME` with a repo that has open issues, and report the repo.
- An issue whose PR is open shows a PR icon (`●`) before the number. This does not block a start. After the start, the session glyph takes its place.
- The backend is chosen once, at launch, from the `multiplexer` setting and the environment. The sandbox writes `"multiplexer": "tmux"` and removes the herdr and herdr app variables. If you change `multiplexer` in the settings menu and then `restart`, sessions leave the sandbox. Set it back to `tmux` first.
- An empty `pane=` or no `--settings` in `claude-calls.log` means dispatch did not use tmux. Hooks are added only on tmux.
- The default mode is `Implement directly`. The Mode radio changes with Left, Right, or Space only when the `Mode` field has focus (`›` before `Mode`).
- Enter starts the session when `Branch` or `Mode` has focus, as Ctrl-S and Ctrl-Enter do. So `s` then Enter starts at once. Use `C-s` in recipes.
- `down` deletes the worktrees, because they are under `$RUN/work`. The evidence files keep the paths.
