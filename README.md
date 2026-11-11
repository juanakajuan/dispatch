# dispatch

dispatch is a terminal app for one GitHub project. It lists the open issues and the pull requests. You pick an issue and start a Claude Code session for it. Each session gets its own git worktree and its own tmux window, and you choose a skill for it.

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
- Skill, chosen from the issue labels. For example `bug` gives `pstack:tdd`. Automatic means no `/skill` prefix.
- Mode: plan first, or implement directly.
- Prompt. You can edit it.

The dialog shows the exact commands. Press Ctrl-S (or Enter on a one-line field) to start.

dispatch then does this:

1. It fetches the default branch.
2. It runs `git worktree add ../<repo>-<issue> -b <branch> origin/<default>`.
3. It opens a tmux window named `#<issue>` that runs `claude [--permission-mode plan] "/<skill> <prompt>"` in the worktree.

If dispatch runs inside tmux, the window goes into your current tmux session. If not, dispatch creates a detached session named `dispatch-<repo>`. In that case `a` suspends dispatch and runs `tmux attach`. Detach with `Ctrl-b d` to come back.

## Keys

| Key | Action |
|---|---|
| `j` / `k`, arrows | Move the selection |
| `1` `2` `3` | Switch tab |
| `s`, Enter | Start a session for the selected issue (or go to its session) |
| Enter on a PR | Go to the issue the PR closes |
| `a` | Attach to the session. If Claude has exited, resume it with `claude --continue` |
| `x` | Stop the session. The window closes. The worktree stays |
| `r` | Refresh |
| `/` | Filter. Enter keeps it, Esc clears it |
| `q`, Ctrl-C | Quit |

In the dialog: Tab and Shift-Tab change the field, Left and Right change the skill or the mode, Esc cancels.

Mouse: a click selects a row or a tab. A double-click on an issue opens the start dialog. The wheel moves the selection.

The Sessions tab shows the last lines of the Claude pane for the selected session. A session that waits for you shows `needs you`. This needs the hooks that dispatch adds (see `detect_waiting`).

## Config

The file is optional. Its path is `$XDG_CONFIG_HOME/dispatch/config.json`, or `~/.config/dispatch/config.json`. Every field is optional. A field you set replaces the default.

```json
{
  "skills": ["pstack:tdd", "pstack:architect", "pstack:technical-writing"],
  "label_skills": [
    { "label": "bug", "skill": "pstack:tdd" },
    { "label": "feature", "skill": "pstack:architect" }
  ],
  "detect_waiting": true,
  "claude_command": ["claude"]
}
```

- `skills` is the list the skill selector cycles through.
- `label_skills` is checked in order. The first label that the issue has wins. Skills named here are added to `skills`.
- `detect_waiting` is on by default. dispatch passes `--settings` with four hooks to `claude`. They set a flag on the tmux window when Claude waits for you. Set it to `false` if the hooks conflict with yours.
- `claude_command` is the program and its leading arguments. dispatch appends its own arguments.

An invalid file stops dispatch at start with a message. Unknown fields are errors.

## How sessions are tracked

dispatch writes no state file. A session is derived from git and tmux each time:

- A branch that dispatch made has `branch.<name>.dispatch-issue` (and `dispatch-skill`) in `.git/config`.
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
