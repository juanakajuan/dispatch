# Issue and PR lists

dispatch shows three tabs: Issues, Pull requests, and Sessions. Each tab is a list of rows in groups. dispatch fetches issues and PRs every 60 s, on `r`, and 5 s after events that change GitHub data. The header shows how old the data is. A user can filter and sort the rows.

## Sub-features

- `lists-tabs` switches tabs with `1` `2` `3` or a click, and shows the item count of each tab after its name, for example ` 2  Pull requests 10`.
- `lists-groups` puts rows in groups with a heading and a count. Issues: `Needs you`, `Working`, `In review`, `Merged`, `Open`. PRs: `Ready for review`, `Draft`, `Merged`, `Closed`. Sessions: `Needs you`, `Working`, `Exited`. An empty group is not shown.
- `lists-freshness` shows `loading…`, `updated Ns ago`, `refreshing…`, or `refresh failed Ns ago` after `<repo> · ` on header line 5, on tabs 1 and 2 only.
- `lists-refresh` fetches now on `r`.
- `lists-filter` keeps the rows that match the `/` text, without case. Issues match on number, title, and labels. PRs match on number, title, and head branch. Sessions match on number, issue title, and branch. Enter keeps the filter. Esc clears it, in the filter input and also from the list.
- `lists-sort` cycles the order of the current tab with `S`, shown as `S sort: <order>` at the right end of the key bar. Rows are sorted inside each group.
- `lists-mouse` selects a row on a click. A double-click acts like Enter on the row: it opens the start dialog on an issue, goes to the issue of a PR, and attaches to a session. A click on the prompt opens the filter, and `esc to clear` at its right end clears a kept filter. The wheel moves the selection.
- `lists-pr-icon` shows the state of the lead PR of each issue in the glyph column before `#N`, and `PR #P open · <checks>` in the row summary.

## How to get to it (user POV)

- Start dispatch in a checkout. The Issues tab opens.
- Press `1`, `2`, or `3`, or click a tab name.

## Driving it with verify.sh

Preconditions:

- A run is up and doctor shows no `FAIL`.

- **Tabs.** Press `2`. Run `$V keys "$RUN" 2`, `$V wait "$RUN" 'enter to go to issue'`, and `$V wait "$RUN" 'updated [0-9]+[smh]? ago'`. The key bar shows `enter to go to issue` only on the PR tab. Line 2 shows ` 2  Pull requests N`. Run `$V shot "$RUN" prs-tab`.
- **Freshness.** Run `$V screen "$RUN" | sed -n 5p`. It shows `<repo> · updated Ns ago` next to the logo. On tab 3 the line shows only `<repo>`.
- **Refresh.** Press `r`. Run `$V keys "$RUN" r` and `$V wait "$RUN" 'refreshing…|updated [0-9]s ago'`. `gh-calls.log` gets a new `gh issue list` and `gh pr list` line.
- **Filter.** On the PR tab, press `/` and type a word from one title. Run `$V keys "$RUN" /` and `$V wait "$RUN" 'enter to keep · esc to clear'`, then `$V type "$RUN" '<word>'`, `$V keys "$RUN" Enter`, and `$V wait "$RUN" '❯ /<word>'` (the prompt line shows the kept filter). Only matching rows stay. Text that matches nothing shows `No pull requests match the filter.` Press Esc, `$V wait "$RUN" '/ to filter pull requests'`, and all rows come back.
- **Sort.** Press `S` on the PR tab. Run `$V keys "$RUN" S` and `$V wait "$RUN" 'S sort: newest'`. Each press moves to the next order: `attention`, `newest`, `oldest`, `recently updated`, `number`, then `attention` again. The Issues tab adds `most comments` after `number`. The Sessions tab has only `attention` and `number`. The selected row stays selected.
- **PR icon.** On the Issues tab, an issue with an open PR shows `●` before `#N`, for example `❯ ● #12 … PR #17 open · ✗ checks failing`. A draft PR shows `◌` and a merged PR shows `✓`. An issue with no PR, or a closed one, shows `·`. A session glyph (`▲`, `●`, `○`) takes the place of the PR mark. The color of `●` follows the checks: green pass, red fail, yellow pending, plain with no checks.
- **Fetch failure.** Not driven by default. To see it, copy `$RUN/bin/gh`, make `gh issue list` exit 1 in it, press `r` on tab 1, then restore the copy. The header shows `refresh failed Ns ago` in yellow, the message row shows `gh failed, showing old data: …`, and the old rows stay. When no rows were loaded before, the message row shows `gh failed: …` and the list is empty.

## Gotchas

- The counts and rows are live GitHub data. Do not assert fixed numbers.
- `up` is ready when the Issues fetch finishes. The PR fetch is separate and can still be running. On tab 2, wait for `updated` before you read the rows.
- If tab 2 shows `No pull requests.` and the message row shows `gh failed: gh failed with code 1: HTTP 502` (or `504`), the repo has too many PRs for dispatch's PR query. GitHub is not down. Use a smaller repo (see [README.md](README.md)).
- The Sessions tab shows no fetch age. Do not wait for `updated` there.
- On PR rows `✓` means checks pass, not merged. A PR in `Ready for review` with `✓` is open. Read the group heading.
- Mouse input is not driven by `verify.sh`. To test a click, send an SGR mouse sequence with `$V tmux "$RUN" send-keys -t <pane> -l $'\e[<0;X;YM'` and the release `m`. Coordinates are 1-based.
- The `.ansi` shots keep the colors. Use them when a claim depends on color, for example a green or red PR icon, or which tab is selected.
