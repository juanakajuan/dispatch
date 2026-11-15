# Settings and keys

A user presses `,` to open the settings menu and `?` to open the keys menu. The settings menu changes `detect_waiting`, `icons`, `multiplexer`, and `claude_command`, and saves each change to `config.json` at once. The keys menu lists every key binding by group. The key bar on the last line shows only the keys that work on the selected row.

## Sub-features

- `keys-bar` shows `<key> to <action>` pairs joined by ` · `, for the selected row only. `? to keys` is always first and `q to quit` last. `m to merge` shows only for a row with an open or draft PR, `x to stop` only for a live session, `a to resume` only for an exited one.
- `keys-menu` (`?`) shows a box titled ` Keys ` with the groups `Navigation`, `Sessions`, `Pull requests`, `View`, `App`, one row per binding, for example `j / down    move down`. `j`/`k` scroll. Every other key is swallowed, so `q` does not quit. `Esc` or `?` closes it.
- `settings-menu` (`,`) shows a box titled ` Settings ` with one row per setting: `<key>  <value>   default: <default>`. The selected row starts with `›`. A note under the rows says when the change applies. `Esc` or `,` closes it.
- `settings-toggle` changes `detect_waiting` (on, off) or `icons` (unicode, nerdfont), or cycles `multiplexer` (`auto`, `tmux`, `herdr`, `herdr-app`), on Enter.
- `settings-edit` opens an inline editor for `claude_command` on Enter. Enter saves, Esc cancels. Words split on spaces, and quotes group them.
- `settings-save` writes `$XDG_CONFIG_HOME/dispatch/config.json`. It changes only the one key, keeps the other keys, and removes a key when the value is the default. A success shows no toast. A failure shows `Saving settings failed: <error>`.

## How to get to it (user POV)

- Press `?` from any tab. The key bar always shows `? to keys`.
- Press `,` from any tab. The key bar does not show `,`. The keys menu lists it under `App`.

## Driving it with verify.sh

Preconditions:

- A run is up and doctor shows no `FAIL`. Set `CFG=$RUN/xdg/dispatch/config.json` and save `jq -c . "$CFG"`. It has `claude_command` and `"multiplexer":"tmux"`.

- **Key bar.** Press `1`. Run `$V wait "$RUN" '\? to keys · s to start session'`. The bar ends with `q to quit`. Press `3` with a live session selected: the bar has `a to attach · x to stop`. Run `$V shot "$RUN" keybar`.
- **Keys menu.** Press `?`. Run `$V keys "$RUN" '?'` and `$V wait "$RUN" ' Keys '`. The screen shows `Navigation`, `Pull requests`, `j / down +move down`, `q / ctrl\+c +quit`, `, +open the settings menu`, and the key bar `j/k to scroll · esc to close`. Run `$V shot "$RUN" keybinds`. Press `q`: the menu stays open. Press `Escape`: ` Keys ` is gone. Open it again with `?` and close it with `?`.
- **Settings menu.** Press `,`. Run `$V keys "$RUN" ,` and `$V wait "$RUN" ' Settings '`. The rows are `› detect_waiting  on   default: on`, `icons  unicode   default: unicode`, `multiplexer  tmux   default: auto`, and `claude_command  $RUN/bin/fake-claude…   default: claude`. The note is `Applies to sessions you start or resume after this change.` The box ends with `j/k move · enter change · esc close` and `Saved to config.json`. Run `$V shot "$RUN" settings`.
- **Toggle and save.** Press Enter. `$V wait "$RUN" 'detect_waiting +off'`. Poll the file with a bounded loop: `for i in $(seq 50); do jq -e '.detect_waiting == false' "$CFG" && break; sleep 0.1; done`. `claude_command` and `multiplexer` stay. Press Enter again: `detect_waiting +on`, and the key is gone from the file (`jq 'has("detect_waiting")'` prints `false`).
- **Icons.** Press `j` then Enter. `$V wait "$RUN" 'icons +nerdfont'`, and `.icons` is `nerdfont` in the file. Close with `,`: header line 6 has Nerd Font glyphs, not `▲`. Open again, select `icons`, and press Enter to go back to `unicode`. The key is gone from the file.
- **Multiplexer.** Select `multiplexer`. The note is `Applies the next time you start dispatch.` Press Enter four times and wait for `multiplexer +herdr `, `herdr-app`, `auto`, then `tmux`. End on `tmux`, and check `.multiplexer == "tmux"` in the file.
- **Editor error.** Select `claude_command` and press Enter. `$V wait "$RUN" 'enter save · esc cancel'`. Run `$V type "$RUN" " 'oops"` and press Enter. `$V wait "$RUN" 'has a quote that never closes'`. The file does not change. Press `Escape` to cancel the edit. The menu stays open with `j/k move · enter change · esc close`. Press `Escape` again to close it.

## Gotchas

- The save is asynchronous and silent. Prove it with a bounded poll on `$CFG`, not with a screen wait.
- Do not save a changed `claude_command`, or the run loses the fake claude. Cancel the editor with `Escape`.
- Do not leave `multiplexer` on anything but `tmux`. It applies at the next start, so a `restart` would start sessions in herdr or the herdr app, outside the sandbox.
- The `claude_command` value is cut with `…`. Match `default: claude`, not the full path.
- The `icons` setting changes the glyphs in every other feature file. Set it back to `unicode` before you drive them.
- `github_refresh` is fixed at 60 s. The menu does not show it.
