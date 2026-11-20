# Start a session

A user selects an open issue and starts a Claude session for it. A dialog shows the branch, the mode, the prompt, and the commands that a start of a new branch runs. On Ctrl-S dispatch makes a git worktree on a new branch, marks the branch with the issue number, and opens a multiplexer window that runs `claude` in the worktree. A user can also select an open PR and start a session for the PR itself, on the PR's own branch. A session has a key: `#N` for an issue, `PR #P` for a PR. The sandbox pins the multiplexer to tmux.

## Sub-features

- `start-open` opens the dialog from the Issues tab with `s`, Enter, or a double-click.
- `start-fields` shows a branch name `<prefix>/<n>-<slug>`, the mode (plan first or implement), and an editable prompt. The prefix comes from the first label that matches: `bug` gives `fix`, `docs` or `documentation` gives `docs`, `chore` gives `chore`, else `feat`. The slug is the first 4 words of the title.
- `start-preview` shows `git worktree add .claude/worktrees/<n> -b <branch> <base>` and the command of the configured harness (`command`, or the harness program such as `codex`) with the prompt under `Runs`. With harness `codex` and plan first it is `codex --sandbox read-only "<prompt>"`. Only Claude has `--settings "<waiting hooks>"`. With the tmux backend and `detect_waiting` on (the sandbox default), the claude line has `--settings "<waiting hooks>"` in place of the hooks JSON. The preview leaves out the `git fetch origin <default>` that a start runs first. When the branch already exists, a real start runs `git worktree add <dir> <branch>` without `-b`, but the preview still shows `-b`. With the herdr backend the first line is `herdr worktree create …` instead.
- `start-run` makes the worktree, the `branch.<b>.dispatch-issue` and `branch.<b>.dispatch-harness` markers, and a tagged tmux window named `#<n>`.
- `start-trust` shows the session as `needs you` while Claude asks to trust the worktree, and `running` after you answer. dispatch sets the waiting flag on the new window, and Claude's `SessionStart` hook clears it.
- `start-plan-mode` adds `--permission-mode plan` to the argv when the mode is plan first.
- `start-idempotent` goes to the existing session instead of making a second worktree. While a start is still running, `s` shows `#N is already starting`.
- `start-pr` opens `Start session for PR #P <title>` from the Pull requests tab with `s`, for an open or draft PR. The branch line is `<head> (PR branch)`, and typing does not change it. The mode is `Implement directly`. The prompt starts `Work on GitHub PR #P: <title>` and tells Claude to read `gh pr view P --comments` and `gh pr diff P`, then fix and push to the PR's branch. `Runs` shows `$ git worktree add .claude/worktrees/pr-P -b <head> origin/<head>`. Ctrl-S makes the worktree `.claude/worktrees/pr-P`, the marker `branch.<head>.dispatch-pr P`, and a tagged tmux window named `PR #P`.
- `start-pr-fork`: for a PR from a fork that lets maintainers push, the branch line is `fork/pr-P (PR branch)`, and `Runs` starts with `$ git fetch dispatch-fork-pr-P refs/heads/<head>:refs/heads/fork/pr-P`, then `$ git worktree add .claude/worktrees/pr-P fork/pr-P`. The start adds the fork remote as in [Merge](./merge.md) `merge-fork`. The prompt does not name the fork's branch.
- `start-pr-refuse`: `s` on a merged or closed PR shows `PR #P is not open.` and opens no dialog. The key bar still shows `s to start session` on that row.
- `start-pr-existing`: `s` on a PR whose head branch already has a session (an issue session or a PR session) goes to that session on the Sessions tab. No dialog opens.
- `start-batch`: with issues marked by Space, `s` opens the batch dialog instead. See [Start several sessions](./batch-start.md).
- `start-cancel` closes the dialog with Esc and changes nothing.
- `start-mouse`: a click on a field label focuses it, a click on a mode option picks it, a click in a text input puts the cursor there, and the `[ Start ]` and `[ Cancel ]` buttons on the last dialog row act as Ctrl-S and Esc.

## How to get to it (user POV)

- On the Issues tab, select an issue and press `s`.
- On the Pull requests tab, select an open or draft PR and press `s`.
- On the Issues tab, select an issue and press Enter.
- Double-click an issue row.

## Driving it with verify.sh

Preconditions:

- A run is up and `$V doctor "$RUN"` shows no `FAIL`.
- The Issues tab lists at least one open issue `#N` that has no session.

- **Pick the issue.** Press `1`. Run `$V keys "$RUN" 1`, then `$V screen "$RUN"`, and pick the `#N` of a row. Move to it with `j`/`k`. The row starts with `❯`. Run `$V shot "$RUN" issues-before`.
- **Open the dialog.** Press `s`. Run `$V keys "$RUN" s` and `$V wait "$RUN" 'Start session for #N '`. The dialog shows `› Branch`, `Mode` with `● Implement directly`, `Prompt` starting `Implement GitHub issue #N:`, and under `Runs` the lines `$ git worktree add .claude/worktrees/N -b <branch> origin/<default>` and `$ $RUN/bin/fake-claude --settings "<waiting hooks>" "Implement GitHub issue #N: …"`. Long lines wrap with a 4-space indent, so match one row at a time, for example `$V wait "$RUN" 'fake-claude --settings "<waiting'`. The last dialog row is `[ Start ]  [ Cancel ]`. The key bar is `ctrl-s to start · tab to next field · esc to cancel`. Run `$V shot "$RUN" start-dialog`.
- **Cancel.** Press Esc. Run `$V keys "$RUN" Escape`. The dialog closes. `$V state "$RUN" after-cancel` shows only the main worktree and `(none)` under markers.
- **Plan mode (optional).** Reopen with `s`, press `Tab` to focus `Mode`, then `Left`. Run `$V keys "$RUN" s Tab Left`. The radio shows `● Plan first, then wait for approval`, the field shows `› Mode`, and the second `Runs` line is `$ $RUN/bin/fake-claude --permission-mode plan` and wraps to `--settings "<waiting hooks>" "Implement …"`. Space also changes the mode.
- **Start.** Press Ctrl-S. Run `$V keys "$RUN" C-s`, `$V wait "$RUN" 'Starting #N|Started #N'`, and `$V wait "$RUN" '▲ 1 needs you' 20`. dispatch moves to the Sessions tab. The `#N` row shows `❯ ▲ #N` (the fake claude runs dispatch's `Stop` hook after 2 s), the tab row shows `3  Sessions 1`, and header line 6 shows `▲ 1 needs you   ● 0 working   ○ 0 exited`. Do not wait for `needs you` alone: the header always shows it, also as `▲ 0 needs you`. Run `$V shot "$RUN" after-start`. A failed start shows `Start #N failed: <error>` for 5 s.
- **Side effects.** Run `$V state "$RUN" after-start`. It shows the worktree `$RUN/work/<repo>/.claude/worktrees/N` on `[<branch>]`, the markers `branch.<branch>.dispatch-issue N` and `branch.<branch>.dispatch-harness <harness>`, and a window `#N` with `tag=$RUN/work/<repo>/.claude/worktrees/N waiting=1`. `grep -cx '.claude/worktrees/' $RUN/work/<repo>/.git/info/exclude` prints `1`. Do not use `git status` as proof of the exclude: a repo that has `.claude/worktrees/` in its `.gitignore` (dispatch does) hides the worktrees without it.
- **Claude argv.** Read `$RUN/evidence/claude-calls.log`. The first line has `cwd=$RUN/work/<repo>/.claude/worktrees/N` and an argv with `--settings` (the hooks JSON) and the prompt as one argument. With plan mode, the argv starts with `--permission-mode plan`. The next lines are `hook SessionStart`, `hook UserPromptSubmit`, and `hook Stop`.
- **Trust question.** Before a start, run `touch $RUN/ask-trust`. Start an issue `#N` as above. Run `$V wait "$RUN" '▲ 1 needs you' 20`, then `$V keys "$RUN" a` to attach. The pane shows `FAKE-CLAUDE Accessing workspace: do you trust this folder?`. `claude-calls.log` has no `hook` line after the argv line yet. Type `yes` and Enter, detach with `C-b d`. The row glyph goes to `●` (header `● 1 working`), then `▲` after 2 s. Run `rm $RUN/ask-trust` after.
- **Start again.** Press `1`, select `#N`, press `s`. The UI goes to the existing session and no dialog opens. `$V state` still shows one worktree for `N`.
- **PR session.** Needs an open PR `#P` from the run's repo whose head branch has no session. Press `2`, select `#P`, press `s`. Run `$V wait "$RUN" 'Start session for PR #P '`. The dialog shows `› Branch` and `<head> (PR branch)`, `● Implement directly`, a prompt that starts `Work on GitHub PR #P: <title>`, and under `Runs` the lines `$ git worktree add .claude/worktrees/pr-P -b <head>` and `origin/<head>`, then `$ $RUN/bin/fake-claude --settings "<waiting` with the prompt. Run `$V shot "$RUN" pr-start-dialog`. Type `zzz` and press `BSpace` twice: the branch line does not change. Press Ctrl-S and run `$V wait "$RUN" '▲ 1 needs you' 20`. The Sessions tab shows the row `❯ ▲ PR #P <title>  PR #P open`. `$V state "$RUN" pr-after-start` shows the worktree `.claude/worktrees/pr-P` on `[<head>]` at the commit of `origin/<head>`, the marker `branch.<head>.dispatch-pr P`, and the window `PR #P` with the worktree tag. The argv in `claude-calls.log` ends with `$'Work on GitHub PR #P: <title>\n\nRead the PR and its comments with `gh pr view P --comments`, and its diff with `gh pr diff P`. This worktree is on the PR\'s branch. …'`.
- **Fork PR session.** Needs an open fork PR with `fork=true edits=true` (README check command). Press `2`, select it, press `s`. The branch line is `fork/pr-P (PR branch)`. `Runs` has `$ git fetch dispatch-fork-pr-P` and wraps to `refs/heads/<head>:refs/heads/fork/pr-P`, then `$ git worktree add .claude/worktrees/pr-P fork/pr-P`. After Ctrl-S, `git -C "$RUN/work/<repo>" config --get-regexp 'dispatch|branch\.fork/'` shows the `remote.dispatch-fork-pr-P.*` lines, `branch.fork/pr-P.remote`, `branch.fork/pr-P.merge`, and `branch.fork/pr-P.dispatch-pr P`. The argv prompt does not contain `<head>`.
- **PR not open.** Press `2`, select a merged or closed PR `#P`, press `s`. The message row shows `PR #P is not open.` and no dialog opens.
- **PR branch has a session.** Start an issue session whose branch is the head branch of an open PR `#P` (the issue row shows `PR #P open`). Press `2`, select `#P`, press `s`. dispatch goes to the Sessions tab with that session selected, and no dialog opens.

## Gotchas

- The live repo can have no open issue. Then this journey is not verified. Use `up --repo OWNER/NAME` with a repo that has open issues, and report the repo.
- An issue whose PR is open shows a PR icon (`●`) before the number. This does not block a start. After the start, the session glyph takes its place.
- The backend is chosen once, at launch, from the `multiplexer` setting and the environment. The sandbox writes `"multiplexer": "tmux"` and removes the herdr and herdr app variables. If you change `multiplexer` in the settings menu and then `restart`, sessions leave the sandbox. Set it back to `tmux` first.
- An empty `pane=` or no `--settings` in `claude-calls.log` means dispatch did not use tmux. Hooks are added only on tmux.
- The default mode is `Implement directly`. The Mode radio changes with Left, Right, or Space only when the `Mode` field has focus (`›` before `Mode`).
- Enter starts the session when `Branch` or `Mode` has focus, as Ctrl-S and Ctrl-Enter do. So `s` then Enter starts at once. Use `C-s` in recipes.
- A PR session window is named `PR #P`, with a space. Quote it: `$V window "$RUN" 'PR #P'`. For `send-keys`, use the target `'drive:=PR #P'`.
- `down` deletes the worktrees, because they are under `$RUN/work`. The evidence files keep the paths.
