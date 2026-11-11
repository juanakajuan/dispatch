# Merge

A user presses `m` twice to merge a PR. The first press says what dispatch expects to do. The second press reads the PR from GitHub again, then merges a clean PR with `gh pr merge`, sends a fix prompt to the running session of a PR with a problem, or starts a new session to fix it. After a direct merge, dispatch stops the session on the PR's branch.

## Sub-features

- `merge-arm` shows the expectation on the first `m`, for example `Press m again to merge PR #P`.
- `merge-cancel` disarms when any other key comes before the second `m`.
- `merge-direct` runs `gh pr merge <P> --<method> --match-head-commit <sha>` for a clean PR, then stops the session on that branch.
- `merge-handoff` sends the running session a prompt that names the problem and the merge command.
- `merge-new-session` starts a session to fix a PR that has a problem and no running session.
- `merge-refuse` says why when checks run, the PR is not open, or GitHub has not decided.
- `merge-method` reads `git config dispatch.mergeMethod`, else the first method the repo allows.

## How to get to it (user POV)

- On the Pull requests tab, select an open PR and press `m` twice.
- On the Issues tab, select an issue that has a PR and press `m` twice.
- On the Sessions tab, select a session that has a PR and press `m` twice.

## Driving it with verify.sh

Preconditions:

- A run is up without `--allow-merge`, and doctor shows no `FAIL`. The guard blocks `gh pr merge`, so no PR merges.
- The Pull requests tab lists an open PR `#P` (state `open` in the detail pane).

- **Arm.** Press `2`, select `#P`, press `m`. Run `$V keys "$RUN" 2`, move with `j`/`k`, then `$V keys "$RUN" m`. The message row shows `Press m again to merge PR #P`, or a problem sentence such as `PR #P has a merge conflict with main. Press m again to …`. Run `$V shot "$RUN" merge-armed`.
- **Cancel.** Press `j` then `k`, then `m` once. The message row shows the first-press text again, not `Checking PR #P`.
- **Direct merge (clean PR).** Press `m` twice. Run `$V keys "$RUN" m m` and `$V wait "$RUN" 'blocked by dispatch verify guard' 15`. The message row shows the gh error. `$RUN/evidence/gh-blocked.log` has `gh pr merge P --<method> --match-head-commit <sha>`. `gh-calls.log` shows `gh pr view P` and `gh repo view` before it. Run `$V shot "$RUN" merge-blocked`.
- **New session (PR with a problem, no session).** Select a PR whose detail pane shows failing checks or a conflict, and that closes an issue `#N`. Press `m`. The message row says `PR #P has … Press m again to start a session for #N that fixes it.` Press `m` again. dispatch starts a session for `#N` on the PR branch. `$V state` shows the worktree on the PR branch, and `claude-calls.log` has a prompt that tells the agent to run `gh pr checkout P`.
- **Handoff (PR with a problem, running session).** After the step above, press `m` twice on the same PR again. The message row names the problem, and `claude-calls.log` has `got: PR #P has …` with the `gh pr merge P --<method>` command.
- **Not open.** Select a merged PR and press `m`. The message row shows `PR #P is not open.`

## Gotchas

- Never use `--allow-merge` on a repo whose PRs matter. With it, a clean PR really merges.
- The guard turns a direct merge into an error toast. dispatch then does not stop the session, so `merge-direct`'s session stop is not proven by this recipe. It needs a sandbox repo and `--allow-merge`.
- A PR can merge or close on GitHub during the run. The first press then shows `PR #P is not open.` Press `r`, wait for `updated`, and pick a PR again.
- The new-session fix needs a PR that closes an issue. A PR from a fork fails, because its branch is not on `origin`.
- The merge method comes from `git config dispatch.mergeMethod` in the run's clone, not in the user's checkout.
