#!/usr/bin/env bash
# Offline tests of verify.sh. Run by scripts/check.sh.
set -euo pipefail
cd "$(dirname "$0")"
# shellcheck disable=SC1091
source ./verify.sh

fails=0
tmp="$(realpath "$(mktemp -d /tmp/verify-test.XXXXXX)")"
eq() { if [[ $2 == "$3" ]]; then echo "ok    $1"; else echo "FAIL  $1"; printf '  want: %q\n  got:  %q\n' "$2" "$3"; fails=1; fi; }
ok() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; fails=1; fi; }

eq "ps_env_lines drops argv and keeps a value with spaces whole" \
  $'HC_LEAK=1\nSSH_CONNECTION=10.0.0.2 50000 10.0.0.1 22\nPATH=/run/bin:/usr/bin\n_=/run/bin/dispatch' \
  "$(echo '/run/bin/dispatch --flag HC_LEAK=1 SSH_CONNECTION=10.0.0.2 50000 10.0.0.1 22 PATH=/run/bin:/usr/bin _=/run/bin/dispatch' | ps_env_lines)"
eq "ps_env_lines prints nothing for a process with no environment" "" "$(echo '/run/bin/dispatch' | ps_env_lines)"

# The probe is bash, not /bin/sleep: macOS hides the environment of its own
# system binaries from ps. The probe stops itself, so it has no child to orphan.
probe_bin="$(realpath "$BASH")"
env -i HC_LEAK=1 'SSH_CONNECTION=10.0.0.2 50000 10.0.0.1 22' PATH=/run/bin:/usr/bin "$probe_bin" -c 'kill -STOP $$' &
pid=$!
trap 'kill -KILL "$pid" 2>/dev/null || true; rm -rf "$tmp"' EXIT
for _ in 1 2 3 4 5 6 7 8 9 10; do [[ -n $(proc_env "$pid") ]] && break; sleep 0.1; done

eq "proc_env reads a live process's environment, one variable per line" \
  $'HC_LEAK=1\nPATH=/run/bin:/usr/bin\nSSH_CONNECTION=10.0.0.2 50000 10.0.0.1 22' \
  "$(proc_env "$pid" | sort)"
eq "proc_exe reads a live process's executable" "$probe_bin" "$(proc_exe "$pid")"

kill -KILL "$pid"; wait "$pid" 2>/dev/null || true
eq "proc_env prints nothing for a process that is gone" "" "$(proc_env "$pid")"
eq "proc_exe prints nothing for a process that is gone" "" "$(proc_exe "$pid")"

long="$tmp/$(printf 'a%.0s' {1..90})"

mkdir -p "$long/dispatch-verify/planted"
touch "$long/dispatch-verify/planted/run.env"
listed="$(env -u DISPATCH_VERIFY_HOME TMPDIR="$long/" ./verify.sh list)"
ok "a long TMPDIR does not move the run root" '[[ $listed != *planted* ]]'
listed="$(DISPATCH_VERIFY_HOME="$long/dispatch-verify" ./verify.sh list)"
ok "DISPATCH_VERIFY_HOME moves the run root" '[[ $listed == "down  $long/dispatch-verify/planted" ]]'

home="$long/home"
status=0
err="$(DISPATCH_VERIFY_HOME="$home" ./verify.sh up 2>&1 >/dev/null)" || status=$?
ok "up refuses a socket path that is too long" '[[ $status == 1 ]]'
ok "the refusal names the limit and the fix" \
  '[[ $err =~ ^verify:\ tmux\ socket\ path\ is\ [0-9]+\ bytes,\ over\ the\ limit\ of\ (103|107):\ "$home"/[0-9-]+/tmux/sock\.\ Set\ DISPATCH_VERIFY_HOME\ to\ a\ shorter\ folder\.$ ]]'
ok "a refused up leaves no run behind" '[[ -z $(ls -A "$home" 2>/dev/null) ]]'

exit "$fails"
