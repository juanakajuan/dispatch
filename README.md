# dispatch

dispatch is a terminal app for one GitHub project. It lists the open issues and the pull requests. You pick an issue and start a Claude Code session for it. Each session gets its own git worktree and its own tmux window.

## Requirements

- `git`
- `tmux`
- `gh`, logged in (`gh auth status`)
- `claude`, the Claude Code CLI
- A Rust toolchain to build it

Run dispatch inside a git checkout that has a GitHub remote called `origin`. The remote must have a default branch ref. If `git symbolic-ref refs/remotes/origin/HEAD` fails, run `git remote set-head origin --auto`.

## Install

```
cargo install --path .
```

Then run `dispatch` in the checkout.

## Use

There are three tabs: Issues, Pull requests, and Sessions. The list is on the left and the details are on the right.

dispatch fetches the issues and the pull requests every 60 seconds. Press `r` to fetch them now. On the Issues and Pull requests tabs, the right end of the tab row shows how old the list is:

| Text | Meaning |
|---|---|
| `loading…` | The first fetch has not finished |
| `updated 12s ago` gray | The last fetch worked. The time counts from when the response arrived |
| `refreshing…` | A fetch is running. The list stays until the new data arrives |
| `refresh failed 40s ago` yellow | The last fetch failed. The list shows the data from the last fetch that worked, and the message row shows the error |

To start a session, select an issue and press `s`, press Enter, or double-click it. A dialog opens with these fields:

- Branch, for example `fix/142-retry-queue-drops-jobs`.
- Mode: plan first, or implement directly.
- Prompt. You can edit it.

The dialog shows the exact commands. Press Ctrl-S (or Enter on a one-line field) to start.

dispatch then does this:

1. It fetches the default branch.
2. It runs `git worktree add ../<repo>-<issue> -b <branch> origin/<default>`.
3. It opens a tmux window named `#<issue>` that runs `claude [--permission-mode plan] "<prompt>"` in the worktree. The prompt has no `/skill` prefix. A SessionStart hook, such as pstack's, picks the workflow.

Each worktree is a new directory. Claude Code asks if you trust the directory the first time, unless you trusted a parent directory before. Attach to the session with `a` to answer. The session does not show `needs you` during this question.

If dispatch runs inside tmux, the window goes into your current tmux session. If not, dispatch creates a detached session named `dispatch-<repo>`. In that case `a` suspends dispatch and runs `tmux attach`. Detach with `Ctrl-b d` to come back.

Each issue row shows the state of the pull request that closes it, between the number and the title:

| Icon | Meaning |
|---|---|
| none | No PR, or only closed PRs |
| `◌` gray | Draft PR |
| `●` yellow | Open PR, checks running |
| `●` red | Open PR, checks failing |
| `●` green | Open PR, checks passing |
| `●` white | Open PR, no checks |
| `✓` purple | Merged PR |

If more than one PR closes the issue, the row shows the most advanced one: merged, then open, then draft. A PR counts when GitHub links it to the issue, for example with `Closes #5` in its description. dispatch loads the 100 newest PRs in any state, so an older merged PR does not show. Press `r` to refresh.

dispatch loads issues and PRs again every 60 seconds and when you press `r`. It also loads them a few seconds after these events, because they often change GitHub data:

- A session starts or stops.
- A session's Claude goes from working to `needs you`, or exits.
- The terminal gets focus back, for example after you merge a PR in the browser. Inside tmux, this needs `set -g focus-events on`.

A burst of events makes one load, at most one each 5 seconds.

## Keys

| Key | Action |
|---|---|
| `j` / `k`, arrows | Move the selection |
| `1` `2` `3` | Switch tab |
| `s`, Enter | Start a session for the selected issue (or go to its session) |
| Enter on a PR | Go to the issue the PR closes |
| `a` | Attach to the session. If Claude has exited, resume it with `claude --continue` |
| `x` | Stop the session. The window closes. The worktree stays |
| `m` | Merge the selected PR, or the PR of the selected issue or session. Press `m` twice to confirm. See [Merge](#merge) |
| `o` | Open the selected issue or PR on GitHub. On the Sessions tab, open the session's issue |
| `r` | Fetch the issues and pull requests now |
| `/` | Filter. Enter keeps it, Esc clears it |
| `<` / `>` | Move the border between the list and the details left or right by 5% |
| `=` | Put the border back where it started |
| `q`, Ctrl-C | Quit |

In the dialog: Tab and Shift-Tab change the field, Left and Right change the mode, Esc cancels.

Mouse: a click selects a row or a tab. A double-click on an issue opens the start dialog. The wheel moves the selection. Drag the border between the list and the details to resize them.

The border is the same on all three tabs. Each side keeps at least 24 columns for the list and 30 for the details. On a terminal too narrow for both, the panes share the width and the border does not move. A resize lasts until you quit. To change where the border starts, set `split` in the config.

The Sessions tab shows the last lines of the Claude pane for the selected session. A session that waits for you shows `needs you`. This needs the hooks that dispatch adds (see `detect_waiting`).

## Merge

Press `m` twice on a PR, on an issue that has a PR, or on a session that has a PR. You do not need a session. The first press says what dispatch expects to do. The second press reads the PR from GitHub again, because the list can be a minute old. Then:

- If the PR is clean, dispatch runs `gh pr merge <n> --<method> --match-head-commit <sha>` itself. No agent turn. The toast shows the result. If a push lands after the check, GitHub refuses the merge. If the repo uses a merge queue, the toast says the PR is in the queue.
- If the PR has a merge conflict, failing checks, is behind a base that must be up to date, or is blocked by a branch rule, an agent fixes it. See [Fix a PR that can not merge](#fix-a-pr-that-can-not-merge).
- If checks are still running, if a blocked PR has no checks yet, or if GitHub has not yet worked out if the PR can merge, dispatch does nothing and says so. Press `m` twice again later.

### Fix a PR that can not merge

If a running session is on the PR's branch, dispatch sends its agent a prompt. The prompt names the problem and the merge method, for example: "PR #12 has a merge conflict with main. Fix it and push. When the checks pass, merge it with `gh pr merge 12 --squash`."

If no running session is on the PR's branch, dispatch starts a new session to fix the PR. A new session is a big step, so it needs its own `m`:

- If the list already shows the problem, the first `m` says so, for example: "PR #12 has a merge conflict with main. Press m again to start a session for #5 that fixes it." The second `m` checks GitHub. If the problem is still there, the session starts. If the PR is now clean, dispatch merges it.
- If the list showed a clean PR but GitHub finds a problem, the toast names the problem and asks you to press `m` once more. Any other key cancels.

The new session is for an issue that the PR closes, on the PR's branch. If the branch is not in your checkout, dispatch fetches it from `origin`. The prompt tells the agent to run `gh pr checkout <n>` first, so it has the latest commits. If a session for that issue exited on the PR's branch, the new Claude runs in that session's worktree.

dispatch can not start a session for a PR that closes no issue, because each session belongs to an issue. A PR from a fork also fails, because its branch is not on `origin`.

dispatch never passes `--delete-branch`, because the session's worktree still has the branch checked out.

### Merge method

The merge method is a setting of the project, kept in the repo's git config. From the checkout:

```
git config dispatch.mergeMethod squash     # merge, squash, or rebase
```

Without the setting, dispatch uses the first method the repo allows, in the order `merge`, `squash`, `rebase`. So a repo that allows one method gets that one. If the setting names a method the repo does not allow, the merge stops with a message that lists the allowed methods. dispatch reads the setting on each merge, so a change needs no restart.

## Colors

dispatch uses the Gruvbox dark colors for text. It paints no background, so the background of your terminal shows through.

## Config

The file is optional. Its path is `$XDG_CONFIG_HOME/dispatch/config.json`, or `~/.config/dispatch/config.json`. Every field is optional. A field you set replaces the default.

```json
{
  "detect_waiting": true,
  "claude_command": ["claude"],
  "split": 42
}
```

- `detect_waiting` is on by default. dispatch passes `--settings` with four hooks to `claude`. They set a flag on the tmux window when Claude waits for you. Set it to `false` if the hooks conflict with yours.
- `claude_command` is the program and its leading arguments. dispatch appends its own arguments.
- `split` is the width of the list as a percent of the window, from 10 to 90. The default is 42. `=` puts the border back here.

An invalid file stops dispatch at start with a message. Unknown fields are errors.

## How sessions are tracked

dispatch writes no state file. A session is derived from git and tmux each time:

- A branch that dispatch made has `branch.<name>.dispatch-issue` in `.git/config`.
- The worktree of that branch is the session's directory.
- A tmux window with the option `@dispatch-worktree` set to that directory is the session's window.

So you can quit dispatch, kill it, or restart the machine, and the sessions are still listed. A start is safe to repeat. dispatch looks at what exists and runs only the missing steps. A second start for the same issue does not make a second worktree.

If a session's window is gone, the session shows as `exited`. If you check out another branch inside the worktree, the session disappears from the list until you switch back.

## Remove a session's worktree by hand

dispatch never deletes worktrees or branches. To remove one, from the main checkout:

```
tmux kill-window -t '#142'        # if the window still exists
git worktree remove ../<repo>-142
git branch -D fix/142-retry-queue-drops-jobs
```

`git branch -D` also removes the markers in `.git/config`, so the session leaves the list. Use `--force` on `git worktree remove` if the worktree has changes you want to drop.
