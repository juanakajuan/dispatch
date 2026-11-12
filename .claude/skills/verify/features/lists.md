# Issue and PR lists

dispatch shows three tabs: Issues, Pull requests, and Sessions, with a list on the left and details on the right. It fetches issues and PRs every 60 s, on `r`, and a few seconds after events that change GitHub data. The tab row shows how old the data is. A user can filter, sort, and resize the panes.

## Sub-features

- `lists-tabs` switches tabs with `1` `2` `3` or a click, and shows the item count of each tab in parentheses, for example `2 Pull requests (10)`.
- `lists-freshness` shows `loading…`, `updated Ns ago`, `refreshing…`, or `refresh failed Ns ago` on tabs 1 and 2.
- `lists-refresh` fetches now on `r`.
- `lists-filter` keeps the rows that match the `/` text, without case. Issues match on number, title, and labels. PRs match on number, title, and head branch. Sessions match on number, issue title, and branch. Enter keeps the filter. Esc clears it.
- `lists-sort` cycles the order of the current tab with `S`, shown as `S sort: <order>`.
- `lists-pr-icon` shows the state of the PR that closes each issue between the number and the title.
- `lists-details` shows the issue body as Markdown, or the PR state, checks, branch, diff, and closed issues.
- `lists-resize` moves the border with `<` and `>` by 5 %, and back with `=`.

## How to get to it (user POV)

- Start dispatch in a checkout. The Issues tab opens.
- Press `1`, `2`, or `3`, or click a tab name.

## Driving it with verify.sh

Preconditions:

- A run is up and doctor shows no `FAIL`.

- **Tabs.** Press `2`, then `1`. Run `$V keys "$RUN" 2`, `$V wait "$RUN" 'enter go to issue'`, and `$V wait "$RUN" 'updated [0-9]+[smh]? ago'`. The key bar shows `enter go to issue` only on the PR tab. Line 2 has `━` under `2 Pull requests (N)`. Run `$V shot "$RUN" prs-tab`.
- **Freshness.** Run `$V screen "$RUN" | head -1`. It ends with `updated Ns ago   <repo>`.
- **Refresh.** Press `r`. Run `$V keys "$RUN" r` and `$V wait "$RUN" 'refreshing…|updated [0-9]s ago'`. `gh-calls.log` gets a new `gh issue list` and `gh pr list` line.
- **Filter.** On the PR tab, press `/` and type a word from one title. Run `$V keys "$RUN" /` and `$V wait "$RUN" 'enter keep +esc clear'`, then `$V type "$RUN" '<word>'`, `$V keys "$RUN" Enter`, and `$V wait "$RUN" '/<word>'` (the key bar starts with the kept filter). Only matching rows stay. Text that matches nothing shows `No pull requests match the filter.` Press Esc to clear, and all rows come back.
- **Sort.** Press `S` on the PR tab. Run `$V keys "$RUN" S` and `$V wait "$RUN" 'S sort: newest'`. Each press moves to the next order: `attention`, `newest`, `oldest`, `recently updated`, `number`, then `attention` again. The selected row stays selected.
- **Details.** On the PR tab, select a PR. The detail pane shows `state`, `checks`, `branch` (`<head> → <base>`), `diff` (`+N -N`), and `closes` for a PR that closes issues. The key column has a width of 9 cells, so the screen shows `closes   #N`. Use `$V wait "$RUN" 'closes +#N'`, not `closes #N`.
- **PR icon.** On the Issues tab, an issue with an open PR shows `●` after `#N`. A draft PR shows `◌` and a merged PR shows `✓`. A closed PR shows no mark. The color of `●` follows the checks: green pass, red fail, yellow pending. Compare with the PR tab: the PR's detail pane lists `closes   #N`.
- **Resize.** Press `>`, then `=`. Run `$V keys "$RUN" '>'`, `$V shot "$RUN" wider-list`, `$V keys "$RUN" =`, `$V shot "$RUN" split-reset`. The `│` border moves right by 5 % of 160 columns (8 columns), then returns.
- **Fetch failure.** Not driven by default. To see it, copy `$RUN/bin/gh`, make `gh issue list` exit 1 in it, press `r` on tab 1, then restore the copy. The tab row shows `refresh failed Ns ago` in yellow, the message row shows `gh failed, showing old data: …`, and the old rows stay. When no rows were loaded before, the message row shows `gh failed: …` and the list is empty.

## Gotchas

- The counts and rows are live GitHub data. Do not assert fixed numbers.
- `up` is ready when the Issues fetch finishes. The PR fetch is separate and can still be running. On tab 2, wait for `updated` before you read the rows.
- If tab 2 shows `No pull requests.` and the message row shows `gh failed: gh failed with code 1: HTTP 502` (or `504`), the repo has too many PRs for dispatch's PR query. GitHub is not down. Use a smaller repo (see [README.md](README.md)).
- The Sessions tab shows no fetch age. Do not wait for `updated` there.
- `S` on the Sessions tab has only `attention` and `number`. `most comments` is on the Issues tab only.
- Mouse input is not driven by `verify.sh`. To test a click, send an SGR mouse sequence with `$V tmux "$RUN" send-keys -t <pane> -l $'\e[<0;X;YM'` and the release `m`. Coordinates are 1-based.
- The `.ansi` shots keep the colors. Use them when a claim depends on color, for example a green or red PR icon.
