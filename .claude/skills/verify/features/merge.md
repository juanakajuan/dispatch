# Merge

A user presses `m` twice to merge a PR. The first press says what dispatch expects to do. The second press reads the PR from GitHub again, then merges a clean PR with `gh pr merge`, sends a fix prompt to the running session of a PR with a problem, or starts a new session to fix it. After a direct merge, dispatch stops the session on the PR's branch.

## Sub-features

- `merge-arm` shows the expectation on the first `m`, for example `Press m again to merge PR #P`.
- `merge-cancel` disarms when any other key comes before the second `m`.
- `merge-direct` runs `gh pr merge <P> --<method> --match-head-commit <sha>` for a PR whose merge state is `CLEAN` or `HAS_HOOKS`. If the PR reads back as merged, it stops the running or needs-you session on that branch. If the PR is still open, the toast is `PR #P is in the merge queue`.
- `merge-handoff` sends the live session on the PR branch a prompt that names the problem and the merge command.
- `merge-new-session` starts a session to fix a PR that has a problem (failing checks, conflict, behind, blocked by a rule, unstable) and no live session. It goes to the Sessions tab.
- `merge-refuse` says why when checks run, the PR is a draft (`PR #P is a draft.`), the PR is not open, or GitHub has not decided.
- `merge-method` reads `git config dispatch.mergeMethod`, else the first method the repo allows.

## How to get to it (user POV)

- On the Pull requests tab, select an open PR and press `m` twice.
- On the Issues tab, select an issue that has a PR and press `m` twice.
- On the Sessions tab, select a session that has a PR and press `m` twice.

## Driving it with verify.sh

Preconditions:

- A run is up without `--allow-merge`, and doctor shows no `FAIL`. The guard blocks `gh pr merge`, so no PR merges.
- The Pull requests tab lists an open PR `#P` (state `open` in the detail pane), and the key bar shows `m merge`. Pick repos with the check command in [README.md](README.md). One repo seldom has both a `CLEAN` PR and a PR with a problem that closes an issue, so the direct merge can need a second run on another repo.

- **Arm.** Press `2`, select `#P`, press `m`. Run `$V keys "$RUN" 2`, move with `j`/`k`, then `$V keys "$RUN" m`. The message row shows `Press m again to merge PR #P`, or a problem sentence such as `PR #P has failing checks. Press m again to start a session for #N that fixes it.` The first press reads only the cached list: `gh-calls.log` gets no `gh pr view P`. Run `$V shot "$RUN" merge-armed`.
- **Cancel.** Press `j` then `k`, then `m` once. The message row shows the first-press text again, not `Checking PR #P`. dispatch is now armed again.
- **Direct merge (`CLEAN` PR).** After **Cancel**, press `m` one time (else press it twice). Run `$V keys "$RUN" m` and `$V wait "$RUN" 'blocked by dispatch verify guard' 15`. The message row shows `Checking PR #P`, then `Merge PR #P failed: gh failed with code 1: blocked by dispatch verify guard: gh pr merge P --<method> --match-head-commit <sha>` on two lines. `$RUN/evidence/gh-blocked.log` has `BLOCKED <time> gh pr merge P --<method> --match-head-commit <sha>`. `gh-calls.log` shows `gh pr view P` and `gh repo view` before it. Run `$V shot "$RUN" merge-blocked` at once.
- **New session (PR with a problem, no session).** Select a PR whose detail pane shows a problem such as `✗ checks failing`, and that closes an issue `#N`. Press `m`. The message row says `PR #P has … Press m again to start a session for #N that fixes it.` Press `m` again. The message row shows `Checking PR #P`, then `PR #P has … Starting a session for #N to fix it.` dispatch goes to the Sessions tab and starts a session for `#N` on the PR branch. `$V state` shows the worktree `.claude/worktrees/N` on the PR head branch and the marker. The argv in `claude-calls.log` has `This worktree is on the branch of PR #P. Run \`gh pr checkout P\` …`.
- **Handoff (PR with a problem, live session).** Stay on the Sessions tab with `#N` selected. Press `m`. The message row shows `Press m again to ask #N's agent to fix and merge it.` Press `m` again and `$V wait "$RUN" "Asked #N's agent to fix and merge its PR"`. `claude-calls.log` has `got: PR #P has … When the checks pass, merge it with \`gh pr merge P --<method>\`.`, and `$V window "$RUN" '#N'` shows `FAKE-CLAUDE got: PR #P …`.
- **Not open.** Select a merged PR. The key bar has no `m merge`. Press `m`. The message row shows `PR #P is not open.`

## Gotchas

- Never use `--allow-merge` on a repo whose PRs matter. With it, a clean PR really merges.
- The guard turns a direct merge into an error toast. dispatch then does not stop the session, so `merge-direct`'s session stop is not proven by this recipe. It needs a sandbox repo and `--allow-merge`.
- A PR can merge or close on GitHub during the run. The first press reads the cached list, so it can still arm. The second press reads the PR again and shows `PR #P is already merged.` or `PR #P is closed.` Press `r`, wait for `updated`, and pick a PR again.
- A direct merge needs merge state `CLEAN`. On a repo that requires a review, an open PR with passing checks is `BLOCKED`, and `m` hands it off with `PR #P is blocked by a branch rule, such as a required review.` instead.
- The new-session fix needs a PR that closes an issue. A PR from a fork fails, because its branch is not on `origin`. A draft PR is refused.
- The new-session fix runs `git fetch origin <head>` and then `git worktree add … origin/<head>`. This needs a clone that tracks every branch. `up` sets this, and doctor checks it. If the start fails with `invalid reference: origin/<head>`, the clone tracks only the default branch.
- The merge method comes from `git config dispatch.mergeMethod` in the run's clone, not in the user's checkout.
