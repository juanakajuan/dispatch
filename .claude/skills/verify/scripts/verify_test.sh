#!/usr/bin/env bash
# Tests for the process readers that `verify.sh doctor` uses. Run by scripts/check.sh.
set -euo pipefail
cd "$(dirname "$0")"
# shellcheck disable=SC1091
source ./verify.sh

fails=0
eq() { if [[ $2 == "$3" ]]; then echo "ok    $1"; else echo "FAIL  $1"; printf '  want: %q\n  got:  %q\n' "$2" "$3"; fails=1; fi; }

eq "ps_env_lines drops argv and keeps a value with spaces whole" \
  $'HC_LEAK=1\nSSH_CONNECTION=10.0.0.2 50000 10.0.0.1 22\nPATH=/run/bin:/usr/bin\n_=/run/bin/dispatch' \
  "$(echo '/run/bin/dispatch --flag HC_LEAK=1 SSH_CONNECTION=10.0.0.2 50000 10.0.0.1 22 PATH=/run/bin:/usr/bin _=/run/bin/dispatch' | ps_env_lines)"
eq "ps_env_lines prints nothing for a process with no environment" "" "$(echo '/run/bin/dispatch' | ps_env_lines)"

sleep_bin="$(realpath "$(command -v sleep)")"
env -i HC_LEAK=1 'SSH_CONNECTION=10.0.0.2 50000 10.0.0.1 22' PATH=/run/bin:/usr/bin "$sleep_bin" 60 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true' EXIT
for _ in 1 2 3 4 5 6 7 8 9 10; do [[ -n $(proc_env "$pid") ]] && break; sleep 0.1; done

eq "proc_env reads a live process's environment, one variable per line" \
  $'HC_LEAK=1\nPATH=/run/bin:/usr/bin\nSSH_CONNECTION=10.0.0.2 50000 10.0.0.1 22' \
  "$(proc_env "$pid" | sort)"
eq "proc_exe reads a live process's executable" "$sleep_bin" "$(proc_exe "$pid")"

kill "$pid"; wait "$pid" 2>/dev/null || true
eq "proc_env prints nothing for a process that is gone" "" "$(proc_env "$pid")"
eq "proc_exe prints nothing for a process that is gone" "" "$(proc_exe "$pid")"

exit "$fails"
