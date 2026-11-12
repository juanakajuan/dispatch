#!/usr/bin/env bash
# Drive the dispatch TUI in an isolated tmux server, for verification.
#
#   verify.sh build                      cargo build, print the binary path
#   verify.sh up [--repo OWNER/NAME] [--size COLSxROWS] [--allow-merge]
#                                        start a run, print RUN=<dir>
#   verify.sh doctor RUN                 read-only health check of a run
#   verify.sh keys RUN KEY...            tmux send-keys (Enter, Escape, C-s, Tab, BTab, j, ...)
#   verify.sh type RUN TEXT              type TEXT literally
#   verify.sh wait RUN REGEX [SECONDS]   wait until the screen matches REGEX (grep -E)
#   verify.sh screen RUN                 print the screen now (no file)
#   verify.sh shot RUN LABEL             save the screen to evidence/NN-LABEL.txt (+ .ansi)
#   verify.sh state RUN LABEL            save git worktrees, dispatch markers, tmux windows
#   verify.sh window RUN NAME            print the pane text of the tmux window NAME (for example '#42')
#   verify.sh client RUN                 attach a headless tmux client (needed for `a` attach)
#   verify.sh tmux RUN ARGS...           run tmux ARGS against this run's server only
#   verify.sh restart RUN                start dispatch again in the drive pane, with the run's PATH
#   verify.sh down RUN                   kill this run's tmux server, delete its scratch, keep evidence
#   verify.sh list                       list runs and whether their tmux server is up
#
# Every run lives in $DISPATCH_VERIFY_HOME/<id> (default /tmp/dispatch-verify).
# Nothing touches the user's tmux server, ~/.config/dispatch, or their checkout.
set -euo pipefail

HOME_DIR="${DISPATCH_VERIFY_HOME:-${TMPDIR:-/tmp}/dispatch-verify}"
ROOT="$(git -C "$(dirname "$(realpath "$0")")" rev-parse --show-toplevel)"

die() { echo "verify: $*" >&2; exit 1; }

load() {
  local run="${1:-}"
  [[ -n $run ]] || die "missing RUN argument (the dir that 'up' printed)"
  [[ -f $run/run.env ]] || die "$run is not a dispatch verify run (no run.env)"
  RUN="$(realpath "$run")"
  # shellcheck disable=SC1091
  source "$RUN/run.env"
}

# Always the run's own server. -S beats $TMUX, so the user's server is never hit.
t() { env -u TMUX -u TMUX_PANE tmux -S "$SOCK" "$@"; }

next_index() { printf '%02d' "$(( $(find "$RUN/evidence" -maxdepth 1 -name '[0-9][0-9]-*.txt' | wc -l) + 1 ))"; }

cmd_build() {
  (cd "$ROOT" && cargo build --quiet) >&2
  echo "$ROOT/target/debug/dispatch"
}

cmd_up() {
  local repo="" size="160x45" allow_merge=0
  while (($#)); do
    case "$1" in
      --repo) repo="$2"; shift 2 ;;
      --size) size="$2"; shift 2 ;;
      --allow-merge) allow_merge=1; shift ;;
      *) die "unknown up option $1" ;;
    esac
  done
  local bin_src="$ROOT/target/debug/dispatch"
  [[ -x $bin_src ]] || die "no binary at $bin_src. Run: verify.sh build"
  local real_gh; real_gh="$(command -v gh)" || die "gh is not on PATH"
  command -v jq >/dev/null || die "jq is not on PATH (the fake claude needs it)"
  gh auth status >/dev/null 2>&1 || die "gh is not logged in (gh auth status)"
  [[ -n $repo ]] || repo="$(cd "$ROOT" && gh repo view --json nameWithOwner -q .nameWithOwner)"

  local id; id="$(date +%Y%m%d-%H%M%S)-$$"
  RUN="$HOME_DIR/$id"
  mkdir -p "$RUN"/{bin,evidence,tmux,work,xdg/dispatch}
  RUN="$(realpath "$RUN")"
  # dispatch reads $XDG_CONFIG_HOME/dispatch/config.json. gh and git read the same
  # base dir, so link every other entry of the real config dir into the run's dir.
  local real_xdg="${XDG_CONFIG_HOME:-$HOME/.config}" entry
  for entry in "$real_xdg"/* "$real_xdg"/.[!.]*; do
    if [[ -e $entry && $(basename "$entry") != dispatch ]]; then ln -s "$entry" "$RUN/xdg/"; fi
  done
  SOCK="$RUN/tmux/sock"
  local name="${repo##*/}"
  CLONE="$RUN/work/$name"

  cp "$bin_src" "$RUN/bin/dispatch"

  # gh guard: real gh for reads, logs every call, blocks writes.
  cat >"$RUN/bin/gh" <<EOF
#!/usr/bin/env bash
# Verification scaffolding from verify.sh up. Real gh for reads; blocks writes.
line="\$(date -Is) gh\$(printf ' %q' "\$@")"
echo "\$line" >>"$RUN/evidence/gh-calls.log"
blocked() { echo "BLOCKED \$line" >>"$RUN/evidence/gh-blocked.log"; echo "blocked by dispatch verify guard: gh \$*" >&2; exit 1; }
case "\${1:-} \${2:-}" in
  "issue list"|"pr list"|"issue view"|"pr view"|"repo view"|"auth status") exec "$real_gh" "\$@" ;;
  "pr merge") if [[ $allow_merge == 1 ]]; then exec "$real_gh" "\$@"; fi; blocked "\$@" ;;
  *) blocked "\$@" ;;
esac
EOF

  # Browser stand-in: gh view --web calls it with the URL.
  cat >"$RUN/bin/browser" <<EOF
#!/usr/bin/env bash
# Verification scaffolding from verify.sh up. Logs the URL, opens nothing.
printf '%s %s\n' "\$(date -Is)" "\$*" >>"$RUN/evidence/browser.log"
EOF

  # Claude stand-in: logs argv, runs dispatch's real --settings hooks, echoes input.
  cat >"$RUN/bin/fake-claude" <<EOF
#!/usr/bin/env bash
# Verification scaffolding from verify.sh up. Stands in for the claude CLI.
log="$RUN/evidence/claude-calls.log"
echo "\$(date -Is) cwd=\$PWD pane=\${TMUX_PANE:-} argv:\$(printf ' %q' "\$@")" >>"\$log"
settings="" prev=""
for a in "\$@"; do [[ \$prev == --settings ]] && settings="\$a"; prev="\$a"; done
hook() {
  [[ -n \$settings ]] || return 0
  local c; c="\$(jq -r --arg e "\$1" '.hooks[\$e][0].hooks[0].command // empty' <<<"\$settings")"
  [[ -n \$c ]] && sh -c "\$c"
  printf '%s hook %s\n' "\$(date -Is)" "\$1" >>"\$log"
}
echo "FAKE-CLAUDE started in \$PWD on branch \$(git branch --show-current)"
hook UserPromptSubmit; echo "FAKE-CLAUDE working"; sleep 2
hook Stop; echo "FAKE-CLAUDE waiting for input (type exit to quit)"
while IFS= read -r line; do
  [[ \$line == exit ]] && { echo "FAKE-CLAUDE exiting"; exit 0; }
  hook UserPromptSubmit
  printf '%s got: %s\n' "\$(date -Is)" "\$line" >>"\$log"
  echo "FAKE-CLAUDE got: \$line"; sleep 1
  hook Stop; echo "FAKE-CLAUDE waiting for input"
done
EOF
  chmod +x "$RUN/bin/"{gh,browser,fake-claude}

  jq -n --arg c "$RUN/bin/fake-claude" '{claude_command: [$c], detect_waiting: true}' >"$RUN/xdg/dispatch/config.json"

  gh repo clone "$repo" "$CLONE" -- --depth 1 --quiet >&2
  # A clone of a fork gets an `upstream` remote that gh resolves by default, and
  # dispatch runs gh without -R. Pin gh to the repo the run is for.
  (cd "$CLONE" && gh repo set-default "$repo" >/dev/null)
  # --depth 1 implies --single-branch. Track every branch, as a normal clone
  # does, so `git fetch origin <pr-branch>` makes origin/<pr-branch> for m's fix start.
  git -C "$CLONE" config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'
  git -C "$CLONE" symbolic-ref refs/remotes/origin/HEAD >/dev/null 2>&1 || git -C "$CLONE" remote set-head origin --auto >/dev/null

  cat >"$RUN/run.env" <<EOF
RUN_ID=$id
REPO=$repo
CLONE=$CLONE
SOCK=$SOCK
BIN=$RUN/bin/dispatch
BIN_SRC=$bin_src
BUILT_REV=$(git -C "$ROOT" rev-parse HEAD)$(git -C "$ROOT" diff --quiet HEAD -- src Cargo.toml Cargo.lock || echo -dirty)
ALLOW_MERGE=$allow_merge
EOF

  local cols="${size%x*}" rows="${size#*x}"
  env -u TMUX -u TMUX_PANE \
    PATH="$RUN/bin:$PATH" GH_BROWSER="$RUN/bin/browser" BROWSER="$RUN/bin/browser" \
    XDG_CONFIG_HOME="$RUN/xdg" TMUX_TMPDIR="$RUN/tmux" \
    tmux -S "$SOCK" -f /dev/null new-session -d -s drive -n dispatch -x "$cols" -y "$rows" -c "$CLONE" "$RUN/bin/dispatch"
  t set-option -g remain-on-exit on
  t set-option -g window-size manual
  t set-option -g history-limit 5000
  local pane; pane="$(t display-message -p -t drive:dispatch '#{pane_id}')"
  echo "PANE=$pane" >>"$RUN/run.env"
  echo "PANE_PID=$(t display-message -p -t "$pane" '#{pane_pid}')" >>"$RUN/run.env"
  PANE="$pane"

  if ! wait_for 'updated [0-9]+[smh]? ago' 30 >/dev/null; then
    cmd_shot_inner "up-not-ready"
    die "dispatch did not finish the first fetch in 30s. Screen saved in $RUN/evidence. Run: verify.sh down $RUN"
  fi
  echo "RUN=$RUN"
}

screen_text() { t capture-pane -p -t "$PANE"; }

wait_for() {
  local re="$1" secs="${2:-20}" end=$((SECONDS + ${2:-20}))
  while ((SECONDS < end)); do
    if screen_text | grep -Eq -- "$re"; then return 0; fi
    sleep 0.2
  done
  echo "verify: timed out after ${secs}s waiting for /$re/" >&2
  screen_text
  return 1
}

cmd_shot_inner() {
  local label="$1" n; n="$(next_index)"
  t capture-pane -p -t "$PANE" >"$RUN/evidence/$n-$label.txt"
  t capture-pane -p -e -t "$PANE" >"$RUN/evidence/$n-$label.ansi"
  echo "$RUN/evidence/$n-$label.txt"
}

cmd_doctor() {
  load "$1"
  local ok=1
  check() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; ok=0; fi; }
  check "tmux server answers on $SOCK" 't list-sessions >/dev/null 2>&1'
  check "drive pane $PANE is alive" '[[ $(t display-message -p -t "$PANE" "#{pane_dead}" 2>/dev/null) == 0 ]]'
  check "pane runs this run's binary" '[[ $(realpath "/proc/$(t display-message -p -t "$PANE" "#{pane_pid}" 2>/dev/null)/exe" 2>/dev/null) == "$BIN" ]]'
  check "drive pane has the gh guard first on PATH" 'tr "\0" "\n" </proc/"$(t display-message -p -t "$PANE" "#{pane_pid}" 2>/dev/null)"/environ 2>/dev/null | grep -q "^PATH=$RUN/bin:"'
  check "binary matches $BIN_SRC (no rebuild since up)" 'cmp -s "$BIN" "$BIN_SRC"'
  check "binary is not older than src/ (built at $BUILT_REV)" '[[ -z $(find "$ROOT/src" "$ROOT/Cargo.toml" -newer "$BIN_SRC" -print -quit) ]]'
  check "gh is logged in, as the drive server sees it" 't run-shell "gh auth status >/dev/null 2>&1 && echo GH_OK" 2>/dev/null | grep -q GH_OK'
  check "clone $CLONE has origin/HEAD" 'git -C "$CLONE" symbolic-ref -q refs/remotes/origin/HEAD >/dev/null'
  check "clone tracks every origin branch" '[[ $(git -C "$CLONE" config --get remote.origin.fetch) == "+refs/heads/*:refs/remotes/origin/*" ]]'
  check "gh in the clone resolves to $REPO (not a fork's parent)" '[[ $(cd "$CLONE" && gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null) == "$REPO" ]]'
  check "last GitHub fetch did not fail" '! screen_text 2>/dev/null | grep -q "refresh failed"'
  check "tab row shows a fetch age (tabs 1 and 2 only)" 'screen_text 2>/dev/null | sed -n 1p | grep -Eq "updated [0-9]+[smh]? ago"'
  echo "repo=$REPO built=$BUILT_REV allow_merge=$ALLOW_MERGE"
  echo "--- screen top and message row"
  screen_text 2>/dev/null | sed -n '1,2p;$p' || true
  ((ok))
}

cmd_state() {
  load "$1"; local label="${2:-state}" n out; n="$(next_index)"
  out="$RUN/evidence/$n-$label.state.txt"
  {
    echo "## git worktree list"; git -C "$CLONE" worktree list
    echo "## dispatch markers"; git -C "$CLONE" config --get-regexp '^branch\..*\.dispatch-issue$' || echo "(none)"
    echo "## tmux windows (id name dead tag waiting)"
    t list-windows -a -F '#{window_id} #{window_name} dead=#{pane_dead} tag=#{@dispatch-worktree} waiting=#{@dispatch-waiting}'
  } >"$out"
  # Keep the index in sync with shots: a .txt file numbers this slot.
  cat "$out"
  echo "$out"
}

# A headless attached client, so `a` (select-window ; switch-client) has a
# client to switch, as for a real user. Its own process group, killed by down.
cmd_client() {
  if [[ -f $RUN/client.pgid ]] && kill -0 -- "-$(cat "$RUN/client.pgid")" 2>/dev/null; then
    echo "client already attached (pgid $(cat "$RUN/client.pgid"))"; return 0
  fi
  setsid bash -c "sleep infinity | script -qfec 'env -u TMUX tmux -S $SOCK attach -t drive:dispatch' /dev/null >/dev/null 2>&1" &
  echo $! >"$RUN/client.pgid"
  local end=$((SECONDS + 5))
  until [[ -n $(t list-clients 2>/dev/null) ]]; do ((SECONDS < end)) || die "client did not attach"; sleep 0.2; done
  t list-clients
}

cmd_down() {
  load "$1"
  [[ $RUN == "$(realpath "$HOME_DIR")"/* ]] || die "refusing to clean $RUN: not under $HOME_DIR"
  t kill-server 2>/dev/null || true
  if [[ -f $RUN/client.pgid ]]; then kill -- "-$(cat "$RUN/client.pgid")" 2>/dev/null || true; rm -f "$RUN/client.pgid"; fi
  rm -rf "$RUN/work" "$RUN/xdg" "$RUN/bin" "$RUN/tmux"
  echo "cleaned $RUN. Evidence kept in $RUN/evidence"
  ls "$RUN/evidence"
}

# Start dispatch again in the drive pane, as a user who quits and reruns it.
# tmux gives a pane that a detached client spawns the PATH of that client, not
# the server's, and that wins over -e. So set the PATH from `up` with env, or
# the gh guard is lost.
cmd_restart() {
  load "$1"
  t respawn-pane -k -t "$PANE" env "$(t show-environment -g PATH)" "$BIN"
  wait_for 'updated [0-9]+[smh]? ago' 30 >/dev/null || die "dispatch did not finish the first fetch in 30s after restart"
  echo "restarted dispatch in $PANE"
}

cmd_list() {
  shopt -s nullglob
  for env in "$HOME_DIR"/*/run.env; do
    local dir; dir="$(dirname "$env")"
    if [[ -S $dir/tmux/sock ]] && env -u TMUX tmux -S "$dir/tmux/sock" list-sessions >/dev/null 2>&1; then
      echo "UP    $dir"; else echo "down  $dir"; fi
  done
}

sub="${1:-}"; shift || true
case "$sub" in
  build) cmd_build ;;
  up) cmd_up "$@" ;;
  doctor) cmd_doctor "$@" ;;
  keys) load "$1"; shift; for k in "$@"; do t send-keys -t "$PANE" "$k"; sleep 0.15; done; sleep 0.3 ;;
  type) load "$1"; t send-keys -t "$PANE" -l "$2"; sleep 0.3 ;;
  wait) load "$1"; wait_for "$2" "${3:-20}" >/dev/null ;;
  screen) load "$1"; screen_text ;;
  shot) load "$1"; cmd_shot_inner "${2:?label}" ;;
  state) cmd_state "$@" ;;
  client) load "$1"; cmd_client ;;
  tmux) load "$1"; shift; t "$@" ;;
  window) load "$1"; t capture-pane -p -t "$(t list-windows -a -F '#{window_id} #{window_name}' | awk -v n="$2" '$2==n {print $1; exit}')" ;;
  restart) cmd_restart "$@" ;;
  down) cmd_down "$@" ;;
  list) cmd_list ;;
  *) sed -n '2,/^set -euo/p' "$0" | grep '^#'; exit 2 ;;
esac
