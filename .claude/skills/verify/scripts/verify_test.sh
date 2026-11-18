#!/usr/bin/env bash
# Offline tests of verify.sh. Run by scripts/check.sh.
set -euo pipefail

V="$(dirname "$(realpath "$0")")/verify.sh"
tmp="$(realpath "$(mktemp -d /tmp/verify-test.XXXXXX)")"
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; fail=1; fi; }

long="$tmp/$(printf 'a%.0s' {1..90})"

mkdir -p "$long/dispatch-verify/planted"
touch "$long/dispatch-verify/planted/run.env"
listed="$(env -u DISPATCH_VERIFY_HOME TMPDIR="$long/" "$V" list)"
ok "a long TMPDIR does not move the run root" '[[ $listed != *planted* ]]'
listed="$(DISPATCH_VERIFY_HOME="$long/dispatch-verify" "$V" list)"
ok "DISPATCH_VERIFY_HOME moves the run root" '[[ $listed == "down  $long/dispatch-verify/planted" ]]'

home="$long/home"
status=0
err="$(DISPATCH_VERIFY_HOME="$home" "$V" up 2>&1 >/dev/null)" || status=$?
ok "up refuses a socket path that is too long" '[[ $status == 1 ]]'
ok "the refusal names the limit and the fix" \
  '[[ $err =~ ^verify:\ tmux\ socket\ path\ is\ [0-9]+\ bytes,\ over\ the\ limit\ of\ (103|107):\ "$home"/[0-9-]+/tmux/sock\.\ Set\ DISPATCH_VERIFY_HOME\ to\ a\ shorter\ folder\.$ ]]'
ok "a refused up leaves no run behind" '[[ -z $(ls -A "$home" 2>/dev/null) ]]'

exit "$fail"
