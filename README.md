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

## Keys

| Key | Action |
|---|---|
| `j` / `k`, arrows | Move the selection |
| `1` `2` `3` | Switch tab |
| `s`, Enter | Start a session for the selected issue (or go to its session) |
| Enter on a PR | Go to the issue the PR closes |
| `a` | Attach to the session. If Claude has exited, resume it with `claude --continue` |
| `x` | Stop the session. The window closes. The worktree stays |
| `m` | Ask the session's agent to merge its open PR. Press `m` twice to confirm |
| `o` | Open the selected issue or PR on GitHub. On the Sessions tab, open the session's issue |
| `r` | Refresh |
| `/` | Filter. Enter keeps it, Esc clears it |
| `q`, Ctrl-C | Quit |

In the dialog: Tab and Shift-Tab change the field, Left and Right change the mode, Esc cancels.

Mouse: a click selects a row or a tab. A double-click on an issue opens the start dialog. The wheel moves the selection.

The Sessions tab shows the last lines of the Claude pane for the selected session. A session that waits for you shows `needs you`. This needs the hooks that dispatch adds (see `detect_waiting`).

## Colors

dispatch uses the Gruvbox dark colors for text. It paints no background, so the background of your terminal shows through.

## Config

The file is optional. Its path is `$XDG_CONFIG_HOME/dispatch/config.json`, or `~/.config/dispatch/config.json`. Every field is optional. A field you set replaces the default.

```json
{
  "detect_waiting": true,
  "claude_command": ["claude"]
}
```

- `detect_waiting` is on by default. dispatch passes `--settings` with four hooks to `claude`. They set a flag on the tmux window when Claude waits for you. Set it to `false` if the hooks conflict with yours.
- `claude_command` is the program and its leading arguments. dispatch appends its own arguments.

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
