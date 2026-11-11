# dispatch, candidate 1: stateless sessions derived from git markers and tmux tags

## Problem

The user wants one terminal app that lists the GitHub issues and pull requests of the repo they stand in. From that app they pick an issue and start a Claude Code session for it in its own worktree and tmux window, with a pstack skill. The shape is not obvious for three reasons. First, a session is spread over three systems that dispatch does not own: a git worktree, a branch, and a tmux window. Any of them can change while dispatch is not running. Second, sessions must survive a crash of dispatch, and a start that runs twice must not make two worktrees. Third, the UI must not freeze while `gh` runs, and `attach` outside tmux must hand the real terminal to `tmux attach` and get it back. The work is greenfield, so Phase A had no code to ground on. I checked real output formats instead: tmux 3.8 `list-windows -a -F` (an unset `@option` prints an empty field), `gh pr list --json` (`statusCheckRollup` mixes `CheckRun` and `StatusContext` items, told apart by `__typename`), `git worktree list --porcelain`, and `claude --help` (`--permission-mode`, `--settings <file-or-json>`, `--continue` exist). The tmux man page confirms that `new-window` with more than one argument execs the command directly, without `sh -c`.

## Usage (caller's view)

The user's view, as the README states it:

```
$ cd ~/src/orbit && dispatch
  1 Issues 9   2 Pull requests 4   3 Sessions 2 ▲1
  #142 Retry queue drops jobs when Redis reconnects  [bug][p1]          3h
  ...
  j/k move  1-3 tabs  s start session  a attach  x stop  r refresh  / filter  q quit
```

- Press `s`, Enter, or double-click on an issue. A dialog opens with the branch (`fix/142-retry-queue-drops-jobs`), the skill (`pstack:tdd`, because of the `bug` label), the mode, and the prompt. The dialog also shows the exact commands it will run. Press Ctrl-S to start.
- dispatch makes `../orbit-142` with `git worktree add ../orbit-142 -b fix/142-... origin/main`. It then opens a tmux window `#142` that runs `claude [--permission-mode plan] "/pstack:tdd Implement GitHub issue #142: ..."`.
- On the Sessions tab, the detail pane shows the last lines of the Claude pane, refreshed each second. `a` attaches. Inside tmux, `a` selects the window. Outside tmux, dispatch suspends and runs `tmux attach`, and the TUI comes back when you detach. `x` kills the window, and the worktree stays. `a` on an exited session runs `claude --continue` in the same worktree.
- Quit dispatch, kill it, or reboot. Run `dispatch` again and the same sessions are there. Nothing to clean up, because dispatch writes no state file.
- `~/.config/dispatch/config.toml` (optional) replaces the label-to-skill table and the skill list.

The internal call sites the sketch is derived from:

```rust
// main.rs: startup facts, resolved once, before the TUI.
let ws = Workspace::discover(&cwd, config.detect_waiting)?;

// main.rs: a worker thread. One call per job, one Msg back.
Job::ObserveSessions => Msg::Sessions(ws.observe().map_err(show)),
Job::Start(req)      => Msg::Started { issue: req.issue, result: ws.start(&req).map_err(show) },

// app.rs: the reducer decides what the keys mean from derived state only.
match current_session(state) {
    Some(s) => match (&s.window, s.state()) {
        (Some(w), SessionState::Running | SessionState::NeedsYou) => vec![Cmd::Attach(w.id.clone())],
        _ => vec![Cmd::Job(Job::Resume(s.clone()))],
    },
    None => { toast(state, "No session for this issue. Press s to start one."); vec![] }
}
```

## Shape

Module map (about 2.3k lines with tests):

| module | owns | public surface |
|---|---|---|
| `domain` | validated types: `IssueNumber`, `PrNumber`, `BranchName`, `SkillName`, `WorktreePath`, `Issue`, `PullRequest`, `Checks` | types and their `parse` |
| `rules` | pure product rules: branch slug, prompt, label-to-skill, `claude` argv, worktree dir, filter | pure functions |
| `config` | optional TOML, validated into `Config` | `load()` |
| `github` | `gh` calls and private wire structs | `fetch_issues`, `fetch_prs` |
| `session` (+ private `git`, `tmux`) | everything about worktrees, branches, windows | `Workspace::{discover, observe, start, resume, stop, tail, attach_plan}`, `Session`, `SessionState` |
| `app` | `State`, `Msg`, `Cmd`, `update` reducer, view helpers | `init`, `update`, `visible_rows`, `current_session` |
| `ui` | drawing, the 16-color palette, hit regions | `draw`, `palette` |
| `main` | terminal, loop, job threads, attach suspend | none |
| `proc` | the one subprocess runner, argv only | `run` |

**Session truth lives only in git and tmux.** A session is a worktree whose branch has a `branch.<b>.dispatch-issue` value in `.git/config`, joined on the worktree path with any tmux window whose `@dispatch-worktree` option equals that path. `Workspace::observe` runs three subprocesses (`git worktree list --porcelain`, `git config --get-regexp`, `tmux list-windows -a -F`) and a pure `join`. `Session::state()` derives `Running`, `NeedsYou`, or `Exited` from the window observation. No field stores state, so no stored state can disagree with the world (single source of truth per invariant). The markers survive a dispatch crash, a tmux server restart, and a reboot. `git branch -D` removes the `branch.<b>` section, so a deleted branch leaves no stale marker. `Sessions = BTreeMap<IssueNumber, Session>` makes "one session per issue" a property of the type, per type-system-discipline.

**Start is a planner plus steps.** `plan_start(observation, request) -> StartPlan` is pure. It reads what exists and returns only the missing steps: `Fetch`, `AddWorktree`, `Mark`, `OpenWindow`. Its doc comment holds the full decision table. A crash between any two steps leaves a state that the next plan recognizes. A worktree at the deterministic dir `../<repo>-<N>` without a marker gets `[Mark, OpenWindow]`. A marked branch without a worktree gets `[AddWorktree create=false, ...]`. A marked branch for the issue wins over an edited branch name, so a retry can not make a second branch. The deterministic dir is a second guard, because `git worktree add` refuses an existing path. The test for convergence is direct: apply each prefix of a plan, observe, and plan again. The result must be the remaining suffix. This applies make-operations-idempotent: derive each step from the state that exists, not from what the last run meant to do.

**The app owns UI state only.** `app::update(&mut State, Msg, &Config) -> Vec<Cmd>` is the only writer of `State` and does no I/O, per model-the-domain (a reducer instead of ad hoc mutations). Its one piece of session-related state is `starting: BTreeSet<IssueNumber>`. That set covers the gap between Ctrl-S and the next observation, and it blocks a second `s` on the same issue in the same process. Selection stores row keys, not indices, so a refresh or filter keeps the same item selected. `Focus` is one enum (`List | Filter | Dialog`), so "dialog open while typing a filter" can not be represented.

**Concurrency.** The main thread owns `State`, the terminal, and stdin. Each `Job` runs on its own short-lived `std::thread` with an owned clone of the immutable `Workspace` and sends exactly one `Msg` on an mpsc channel. `Inflight` allows one job per kind, so a slow `gh` never stacks up. There is no lock and no shared mutable object anywhere, per separate-before-serializing-shared-state: the workers publish independent results and the reducer merges them at the read boundary. There is deliberately no input thread. A thread blocked in `crossterm::event::read` would steal keys from `tmux attach` while the TUI is suspended, so the main loop polls input itself with a 100 ms timeout.

**Boundaries.** `gh` JSON, porcelain text, `git config` lines, the tmux format output, the config file, and the dialog's branch text are each parsed into domain types in one function, per boundary-discipline. Wire structs are private to `github`, and `git` and `tmux` are private submodules of `session`. An agent can not import them from `app`, so the "importable internals" red flag can not happen. `Cmd` is split into `Job` (worker) and `Attach`/`Quit` (main thread), so no `unreachable!` arm exists.

**Interface depth.** `Workspace` has seven methods. Behind them sit worktree porcelain parsing, marker read and write, tmux placement (inside vs detached session), window tagging, `remain-on-exit`, crash-convergent planning, and attach mechanics. The reducer never learns that git config or tmux user options exist. `AttachPlan` is the one place where placement leaks out, as data, because only the shell can suspend the terminal.

**Waiting detection is cheap here, so it is in.** The window is already observed through user options. When `detect_waiting` is on (default), `claude` gets `--settings` with four hooks. `Notification` and `Stop` set `@dispatch-waiting` on `$TMUX_PANE`, and `UserPromptSubmit` and `PreToolUse` clear it. The cost is one more format field and one JSON constant. There is no extra polling and no transcript parsing.

**Colors.** `ui::palette` is the only place that builds a `Style`. It uses foreground ANSI colors only, never a background, and `REVERSED` for selection, so the terminal theme applies everywhere.

**Not done.** No state file. No PID tracking. dispatch does not delete worktrees or branches. No multi-repo view. No GitHub writes; Claude opens the PR.

## Synthesis decision

Four candidates ran (Opus, Fable, Sonnet, Haiku). All four chose the same whole shape: no state file, sessions derived from git and tmux, a pure reducer, std threads with one channel. This document is candidate 1 (Opus), the base. An independent judge (Opus) scored it 27 of 30, ahead of 25, 21, and 17, and the orchestrator's own reading agreed. It won because every step of start is pure, planned, and convergent, because it keeps the skill after a crash, and because the main thread owns stdin.

The items below override the text and sketch where they differ.

Grafts:

- **From candidate 2.** `Job::Stop` and `Job::Resume` take an `IssueNumber` and observe again before they act. A stale row can not aim a command at a window id that tmux reused. The config file is JSON (`serde_json`), so the `toml` crate goes. A `TestBackend` test asserts that every drawn cell has `Color::Reset` as its background.
- **From candidate 3.** One `BINDINGS` table drives both key handling and the key bar. `proc` gets a fake seam so the step runner in `Workspace::start` has a test with no real `git` or `tmux`.
- **From candidate 4.** Start, stop, and resume run on one serial writer thread, because two concurrent `git config` writes can fail on the `.git/config` lock (inferred, not measured). The worktree parser reads `prunable`, so a worktree directory that was deleted by hand is not a session.

Facts measured on tmux 3.8 with a throwaway server (`tmux -L`), which change the sketch:

- A chained `; set-option -w` after `new-window -d` tags the current window, not the new one. `OpenWindow` is therefore two calls: `new-window -d -P -F '#{window_id}' ...`, then `set-option -w -t <id>` for `@dispatch-worktree` and `remain-on-exit on`. A crash in the gap leaves an untagged window. This is accepted.
- `new-window -- <argv...>` runs the argv without a shell. A prompt with quotes, `$HOME`, and backticks arrived unchanged.
- `remain-on-exit on` keeps the dead pane, `#{pane_dead}` reports it, and a `-n` window name stays.
- The tmux man page treats an argument that ends in `;` as a command separator. The tmux module must escape a trailing `;` in each argv item as `\;`. Verify this on a real window.

Decisions on the open questions:

- `Fetch` is not fatal. On failure the start continues from the local `origin/<default>` and the app shows a warning.
- `detect_waiting` defaults to on. That Claude merges `--settings` hooks with the user's own hooks is inferred from the documented settings merge. Confirm it in one real session.
- Use `tui-textarea` only if it builds with the current `ratatui`. If not, write a minimal multi-line editor in `ui`.

Rejected: the fixed-path and `pane_start_path` link of candidate 2 (a shell the user opens in the worktree becomes the session, and `x` kills it), the input thread of candidates 3 and 4 (it takes keys from `tmux attach`), and index-based selection (candidate 4).

## Tradeoffs accepted

- We accept three subprocesses every 2 s (about 10 ms each, a guess not measured) in exchange for no state file that can drift from git and tmux.
- We accept that session identity follows the branch, not the directory. If Claude or the user checks out a different branch inside the worktree, the session disappears from the list until the branch is switched back. In exchange, `git branch -D` cleans up for free and the skill is recoverable after a crash.
- We accept splitting the start into `Fetch` + `git worktree add -b` + `git config` + tmux, instead of one shell line, in exchange for a planner that can resume from any partial state.
- We accept that a tagged window can be orphaned if its worktree is deleted by hand. The join ignores it, and the user sees it only in tmux.
- We accept `remain-on-exit on` for session windows. A dead pane stays until `x` or a resume, in exchange for the final screen of Claude staying readable in the detail pane.
- We accept a thread per job instead of a pool or an async runtime. At most about six threads exist, and the dependency list stays short.
- We accept `tui-textarea` as a dependency, in exchange for multi-line prompt editing with a cursor without writing an editor.

## Alternatives considered

- **A state file (`.git/dispatch/sessions.json`) as truth, reconciled against tmux.** Lost because it is a second copy of facts git and tmux already hold. Hand-deleted worktrees, a killed tmux server, or two dispatch instances make it drift, and then it needs a lock and a reconcile step that is the same join as this design anyway. Its interface looks simpler (`load`, `save`), but callers then have to know when the file is stale.
- **Identity by naming convention only (worktree dir `<repo>-<N>`, window name `#N`), no markers.** Lost because a user's own worktree or window with a matching name becomes a false session, and the selected skill is lost after a crash. The deterministic dir stays in this design as a second guard, not as identity.
- **Claude's own session files (`~/.claude/projects/*.jsonl`) for state and waiting detection.** Lost because the format is private to Claude Code and can change in any release. The hooks are a documented interface.
- **tokio with `tokio::process` and an async event stream.** Lost on dependency weight and on the attach suspend. The suspend still needs the main thread to own stdin, so async buys nothing for about six concurrent jobs.
- **A dedicated input thread (the common ratatui pattern).** Lost because it steals stdin from `tmux attach` while suspended, as described above.
- **One `sh -c` string for the tmux command.** Lost because the prompt is user-editable free text. Argv exec removes quoting as a class of bug.

## Implementation reconciliation

Each item lists a change from the sketch and the reason.

**Config**
- `label_skills` is an array of `{label, skill}` objects, not an object map. Reason: table order is the priority, and a JSON object does not keep order in `serde_json`.
- `Config` gains `claude_command: ClaudeCommand { program, args }`. Reason: a program plus args makes an empty command unrepresentable. `config::parse(text, path)` is pure and tested. `load()` only reads the file. The file uses `deny_unknown_fields`, so a typo is an error.
- `Workspace::discover(cwd, &Config)` replaces `discover(cwd, detect_waiting)`, because it also needs `claude_command`. `Layout` drops `place` and `base` (only `Workspace` uses them) and gains `claude`.

**Session**
- `Workspace::start` returns `StartOutcome { session, warnings }`. Reason: a failed `Fetch` is a warning by decision, and the warning must reach the toast. `Msg::Started` carries the outcome.
- `Job::Stop` and `Job::Resume` carry an `IssueNumber`, and `Workspace::stop` and `resume` take one and observe again first (the candidate 2 graft). The sketch's `Session` argument is gone.
- `WorktreePath::planned(PathBuf)` is new. Reason: `plan_start` is pure and the target dir does not exist yet, so `from_observed` (which canonicalizes) cannot build it. The planner builds it from the canonical repo root. `repo_root` is canonicalized in `discover` and uses `--git-common-dir`, so running dispatch inside a linked worktree still names siblings `<repo>-<n>`.
- `OpenWindow` is two tmux calls, as the measured fact required. The step doc comment says so.
- A tagged window whose worktree directory is gone is dropped in `parse_windows` (it cannot canonicalize), not in `join`. `join` still ignores a window with no matching worktree.
- `SetupError::NotARepo` and `NoOriginHead` carry the tool message. `WindowId::parse` is public so tests in other modules can build ids.
- `rules::continue_args` is new, for resume. `BranchName::numbered` is new, as the valid fallback for the branch generator.

**App**
- `app::init(now, &Config, Context)`. `Context { repo_name, base }` lives in `State` for the dialog's "Runs" preview, which needs the repo dir name and the base ref.
- `toast` is `Toast { text, shown_at: Option<Instant> }`. Reason: key messages carry no clock, so the next tick stamps the time. `Due.tail` is `Option<Instant>`, where `None` means "now", so a changed selection loads its tail on the next tick.
- The Sessions tab lists issues in `starting` as rows, so the row shows while the worktree is made. A finished start inserts its session into `State.sessions` at once, so the row does not blink until the next observation. Every observation still replaces the map whole.
- The key bar entries live in `Binding.bar` (a slice, each with a `When`), so one binding can show "a attach" or "a resume" by session state. Enter on the Sessions tab attaches.

**proc**
- The fake seam is a `cfg(test)` thread-local script that `proc::run` consults. Reason: signatures stay as in the sketch and no trait or parameter travels through `git` and `tmux`. Each test runs on its own thread, so scripts do not leak.

**Lints**
- `clippy.toml` sets `allow-unwrap-in-tests` and `allow-expect-in-tests`. The `deny` lints in `Cargo.toml` are unchanged, and the lib code has no `unwrap` or `expect`. No `#[allow]` exists in the source.

**ui and main**
- `tui-textarea` builds with `ratatui` 0.29 and is used as sketched. The dialog clones each `TextArea` per frame to apply palette styles, because `draw` only has `&State`.
- `main` calls `terminal.clear()` after `ratatui::init()`. Measured: after a `kill -9`, the next run drew over the old screen, because the first draw skips blank cells.
- A panic hook also turns mouse capture off.

**Measured in the smoke run (tmux 3.8, a private server)**
- `gh pr list --state all --limit 100 --json ...statusCheckRollup` against `sharkdp/hyperfine` returned HTTP 504 once and 558 KB on the next call. The app shows the error in the tab bar, keeps old data, and retries on the 60 s timer or `r`. The limit stays at 100.
- A prompt that ends in `;` reached the fake `claude` as one argument with the `;` intact, through `escape_argv`. Without it, tmux treats the `;` as a separator (the sketch note held).
- `x` closed the window and kept the worktree. `a` on the exited session respawned it with `--continue`. `a` on a live session selected the window. `git branch -D` removed the markers.
- Not verified: that real `claude` merges the `--settings` hooks with the user's own hooks, and that the hooks set `@dispatch-waiting` in a real session. `detect_waiting` stays on by default as decided. The README says how to turn it off.


## Open questions and risks

- Does a chained `; set-option -w @dispatch-worktree <dir>` in the same tmux invocation target the window that `new-window -d` just made? The man page shows `new-window ; split-window -d` acting on the new window, but not with `-d` on `new-window`. If not, `open_window` uses a second call with `-t <id>`. A crash in the few milliseconds between the two calls would leave an untagged window. The next start would then open a second Claude in the same worktree. Is that acceptable, or should the window name be a fallback tag?
- Do `--settings` hooks merge with the user's own hooks, or replace them? If they replace them, `detect_waiting` must default to off. This needs one manual run before the default is final.
- Should the `Fetch` step be fatal when offline, or fall back to the local `origin/<default>` with a warning? The sketch makes it fatal for simplicity.
- Sessions for closed issues: the issue list holds only open issues, so a session row for a closed issue shows the branch instead of a title. Is that enough, or should dispatch fetch closed issues that have sessions?

## Next implementation step

Write `session::join` and `session::plan_start` with their table tests, together with the `git` and `tmux` parsers and fixtures captured from real output, before any UI code.
