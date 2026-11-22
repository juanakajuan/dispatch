# Settings and keys

A user presses `,` to open the settings menu and `?` to open the keys menu. The settings menu changes `detect_waiting`, `icons`, `multiplexer`, `harness`, `command`, and `remove_on_merge`, and saves each change to `config.json` at once. The keys menu lists every key binding by group. The key bar on the last line shows only the keys that work on the selected row.

## Sub-features

- `keys-bar` shows `<key> to <action>` pairs joined by ` · `, only for actions that work on the selected row, with one hint per action. `? to show keys` is always first and `q to quit` last. An empty tab shows only `? to show keys · q to quit`. A key is written as the keys menu writes it (`ctrl+s`, `down/up`). `enter to go to issue` shows only on a PR that closes an issue. `space to mark` shows only on an issue with no session. `d to show diff` shows only when the session has changes. `m to merge` shows only for a row with an open or draft PR, `x to stop` and `t to send message` only for a live session, and `a to resume` only for an exited one. The prompt placeholder follows the same rule.
- `keys-menu` (`?`) shows a box titled ` Keys ` with the groups `Navigation`, `Sessions`, `Pull requests`, `View`, `App`, one row per binding, for example `j/down     move down`. `j`/`k` scroll. Every other key is swallowed, so `q` does not quit. `Esc` or `?` closes it.
- `settings-menu` (`,`) shows a box titled ` Settings ` with one row per setting: `<key>  <value>   default: <default>`. The selected row starts with `›`. A note under the rows says when the change applies. `Esc` or `,` closes it.
- `settings-toggle` changes `detect_waiting` (on, off), `icons` (unicode, nerdfont), `remove_on_merge` (off, on), or cycles `multiplexer` (`auto`, `tmux`, `herdr`, `herdr-app`), on Enter.
- `settings-edit` opens an inline editor for `command` on Enter. It shows the command that runs now (the harness program when there is no override). Enter saves, Esc cancels. Words split on spaces, and quotes group them.
- `keys-mouse`: a click on a key bar entry or on `S sort: <order>` presses that key. A key cut at a narrow width is not clickable. Each menu box has `esc ✕` in its top border, which closes it. A click on a settings row selects it, and a second click changes it as Enter does. A click on a keys menu entry closes the menu and runs that binding. The wheel moves through the settings and scrolls the keys menu.
- `settings-save` writes `$XDG_CONFIG_HOME/dispatch/config.json`. It changes only the one key, keeps the other keys, and removes a key when the value is the default. A success shows no toast. A failure shows `Saving settings failed: <error>`.

## How to get to it (user POV)

- Press `?` from any tab. The key bar always shows `? to show keys`.
- Press `,` from any tab. The key bar does not show `,`. The keys menu lists it under `App`.

## Driving it with verify.sh

Preconditions:

- A run is up and doctor shows no `FAIL`. Set `CFG=$RUN/xdg/dispatch/config.json` and save `jq -c . "$CFG"`. It has `command` and `"multiplexer":"tmux"`.

- **Key bar.** Press `3` with no sessions: the bar shows only `? to show keys · q to quit` and the prompt shows `/ to filter sessions`. Press `1`. Run `$V wait "$RUN" '\? to show keys · s to start session'`. The bar ends with `q to quit`. Press `3` with a live session selected: the bar has `a to attach · x to stop · t to send message` (no `enter`), and the prompt shows `/ to filter sessions · a to attach`. Run `$V shot "$RUN" keybar`.
- **Keys menu.** Press `?`. Run `$V keys "$RUN" '?'` and `$V wait "$RUN" ' Keys '`. The screen shows `Navigation`, `Pull requests`, `j/down +move down`, `q/ctrl\+c +quit`, `, +open the settings menu`, and the key bar `j/k to scroll · esc to close`. Run `$V shot "$RUN" keybinds`. Press `q`: the menu stays open. Press `Escape`: ` Keys ` is gone. Open it again with `?` and close it with `?`.
- **Settings menu.** Press `,`. Run `$V keys "$RUN" ,` and `$V wait "$RUN" ' Settings '`. The rows are `› detect_waiting   on   default: on`, `icons            unicode   default: unicode`, `multiplexer      tmux   default: auto`, `harness          claude   default: claude`, `command          $RUN/bin/fake-claude…   default: claude`, and `remove_on_merge  off   default: off`. Names are padded to one width, so match with ` +` between the words, for example `$V wait "$RUN" 'icons +unicode +default: unicode'` and `$V wait "$RUN" 'remove_on_merge +off +default: off'`. The note is `Applies to sessions you start or resume after this change.` The box ends with `Saved to config.json`, and has no key hints: the key bar is `j/k to move · enter to change · esc to close`, and the prompt line has no hint. Run `$V shot "$RUN" settings`.
- **Toggle and save.** Press Enter. `$V wait "$RUN" 'detect_waiting +off'`. Poll the file with a bounded loop: `for i in $(seq 50); do jq -e '.detect_waiting == false' "$CFG" && break; sleep 0.1; done`. `command` and `multiplexer` stay. Press Enter again: `detect_waiting +on`, and the key is gone from the file (`jq 'has("detect_waiting")'` prints `false`).
- **Icons.** Press `j` then Enter. `$V wait "$RUN" 'icons +nerdfont'`, and `.icons` is `nerdfont` in the file. Close with `,`: header line 6 has Nerd Font glyphs, not `▲`. Open again, select `icons`, and press Enter to go back to `unicode`. The key is gone from the file.
- **Multiplexer.** Select `multiplexer`. The note is `Applies the next time you start dispatch.` Press Enter four times and wait for `multiplexer +herdr `, `herdr-app`, `auto`, then `tmux`. End on `tmux`, and check `.multiplexer == "tmux"` in the file.
- **Harness.** Select `harness` (three `j`) and press Enter. `$V wait "$RUN" 'harness +codex'`. The `command` row now shows `codex` and the file has `"harness":"codex"` and no `command`. Press Enter three more times to cycle `opencode`, `antigravity`, and back to `claude`: the file loses `harness`.
- **Editor error.** Select `command` and press Enter. `$V wait "$RUN" 'enter to save · esc to cancel'`. Run `$V type "$RUN" " 'oops"` and press Enter. `$V wait "$RUN" 'has a quote that never closes'`. The file does not change. Press `Escape` to cancel the edit. The menu stays open with the key bar `j/k to move · enter to change · esc to close`.
- **Remove on merge.** Select `remove_on_merge` and press Enter. `$V wait "$RUN" 'remove_on_merge +on'`. Poll `$CFG` with a bounded loop: `for i in $(seq 50); do jq -e '.remove_on_merge == true' "$CFG" && break; sleep 0.1; done`. Press Enter again: `$V wait "$RUN" 'remove_on_merge +off'`, and poll until `jq 'has("remove_on_merge")' "$CFG"` prints `false`. Press `Escape` to close the menu.

## Gotchas

- The save is asynchronous and silent. Prove it with a bounded poll on `$CFG`, not with a screen wait.
- Do not save a changed `command`, or the run loses the fake claude. Cycling `harness` also clears `command`, so do it only in a run that you drop after (the `harness` journey in sessions.md does). Cancel the editor with `Escape`.
- Do not leave `multiplexer` on anything but `tmux`. It applies at the next start, so a `restart` would start sessions in herdr or the herdr app, outside the sandbox.
- The `command` value is cut with `…`. Match `default: claude`, not the full path.
- The `icons` setting changes the glyphs in every other feature file. Set it back to `unicode` before you drive them.
- `github_refresh` is fixed at 60 s. The menu does not show it.
