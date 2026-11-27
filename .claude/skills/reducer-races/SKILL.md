---
name: reducer-races
description: Check that dispatch never sends a second start for a session key, under every order of user keys and clicks, late job results, and GitHub changes, up to 9 events. Use it when a change touches starting a session, the `m` merge flow, `armed`, the start dialog, `starting`, or how job results land in `app::update`.
---

# Reducer races

## The promise

dispatch never sends a `WriteJob::Start` for a session key that already has a session or a start in flight.

If dispatch breaks this, the second start reaches `ws.start`, which sees the live window and returns `AlreadyRunning`. The user then sees "Started #3." The second start's prompt and branch are dropped without a message. For example, a fix session for a PR that can not merge never gets its fix prompt, but dispatch still puts the PR in `handed_off`.

## How it is checked

`src/app/tests/races/` runs the real reducer (`app::update`) with no model of it. It starts from the `app()` test fixture: issues #3, #2, #1, PR #10 (head `fix/3-x`, closes #3), and PR #11. It tries every sequence of these events, breadth first, up to `DEPTH` (9) events:

- The user presses `m`, `s`, `j`, or `k` in the list, clicks a tab or a drawn row, or confirms or closes the start dialog. Clicks use the hit map from a real `ui::draw`, as `main.rs` does.
- The writer runs its next write job, in order, as the one writer thread in `src/jobs.rs` does (`races/world.rs`, `run_write`).
- Any pending read job finishes, in any order (`run_read`).
- An hour passes (`Msg::Tick`).
- GitHub changes: an open PR starts to conflict, or the agent of a handed-off PR merges it.

A sequence that reaches a state seen before is dropped. `Run::fingerprint` lists the fields that make up a state. A pass means that every sequence up to 9 events from this fixture keeps the promise.

## Run it

```sh
cargo test --release races -- --ignored --nocapture
```

The test is `#[ignore]` so that `scripts/check.sh` stays fast. It takes about 15 to 60 seconds in release mode, plus the release build the first time. A debug build is 10 times slower. A pass prints `checked N states, every sequence of up to 9 events`. A failure prints the shortest counterexample, one line per event, and then the broken promise.

## When to run it

Run it when a change touches any file in the coverage list below, or adds a `Msg`, a `Focus`, or a key in the list that can lead to a start.

## Read a failure

Replay the counterexample by hand in a unit test in `src/app/tests/` with `press`, `click`, and the `Msg` that each "The writer runs" or "read job" line sends. If the real reducer sends the second start, the failure is a real bug. Report it in the counterexample's words, and do not loosen the check.

The failure is a modeling error only when a step can not happen in the real app. One example is a `World` result that `ws.start` or `merge::job::merge` can never return. Fix `races/world.rs` to match the real job in that case, and name the job's code in the fix.

A new key, `Msg`, or job that the explorer does not model panics with "model it before adding that key". Add it to `Event` and `World`, and keep each result true to the job in `src/jobs.rs`.

## Known findings (open, at the sync commit)

The check fails on the current code. It shows only the shortest counterexample, so the second finding appears only after the first is fixed.

1. **A stale armed fix starts a second session.** The user presses `m` and `m` on #3 (PR #10), then `s`, so the start dialog for #3 opens. The merge job finds a conflict, and `handoff` arms `Armed::Fix` for #3, because the dialog has not started #3 yet. The user confirms the dialog, so #3 starts on `fix/3-retry-queue-drops-jobs`. A click on the Issues tab does not clear `armed`. The next `m` reaches `merge_key`, which calls `start_fix` for #3 on `fix/3-x` with no check of `starting`. (8 events.)
2. **The start dialog starts a key that a fix session took.** PR #10 shows a conflict in the list, so `m` `m` asks with `Consent::StartFixSession`. The user opens the dialog for #3 with `s`. The merge job's `handoff` starts a fix session for #3, because `fixer` saw #3 free. The user confirms the dialog. `confirm` in `src/app/dialog.rs` calls `begin_start` for #3 with no check of `starting`. (8 events.)

Both have one root cause: a start decided earlier is sent without checking `starting` and `sessions` again at send time. A check in `start_fix` and in `confirm` closes both. With those two checks as a scratch change, the run passed on 36974 states.

When a finding is fixed, remove it from this list in the same commit.

## Shown to fail

With both scratch checks in place, the explorer caught this break in 9 events: `fixer` in `src/app/merge.rs` no longer skips keys in `state.starting`. A new property or a new event needs its own such break, run once and reverted.

## Scope and limits

- One fixture: two PRs and three issues, one config (Claude harness, default settings). Other fixtures are not checked.
- At most 9 events. A race that needs more is not seen.
- A read job's result is the world at the time it finishes, not at the time it starts. Stale reads that started before a write and land after it are not modeled.
- No error results: every job succeeds, except that the merge job refuses a merged PR. Failed starts, failed merges, `gh` errors, and remove refusals are not explored.
- Not explored: typing in the dialog or filter, the batch start, `D`, `c`, `x`, `t`, settings, and mouse clicks other than tabs and rows.
- The merge job's methods and draft step are fixed: rebase, never a draft.

## Coverage and sync

Last confirmed that `races/` describes the code at commit `1bf0036`.

The check covers:

- `src/app/merge.rs`: `merge_key`, `fixer`, `handoff`, `start_fix`, `merged`, `merge_result`, `handed_off_merged`
- `src/app/sessions.rs`: `plan_start`, `start_key`, `begin_start`, `started`
- `src/app/dialog.rs`: `confirm`
- `src/app/mod.rs`: `update` and its `armed` take
- `src/app/mouse.rs`: `click`
- `src/jobs.rs`: the writer order that `races/world.rs` copies
- `src/session/plan.rs`: `plan_start`'s `AlreadyRunning` row, which `World::run_write` copies
- `src/merge/job.rs`: `merge` and `finish`, which `World::merge` copies

To see whether the check needs another look, list the covered files that changed since the sync commit:

```sh
git log --oneline 1bf0036..HEAD -- src/app/merge.rs src/app/sessions.rs src/app/dialog.rs src/app/mod.rs src/app/mouse.rs src/jobs.rs src/session/plan.rs src/merge/job.rs
```

Move the sync commit forward only after you have read each change and confirmed that `races/world.rs` and `Event` still describe it. To bring the check back in line after changes that skipped this step, use `/maintain-formal-verification-skill`.
