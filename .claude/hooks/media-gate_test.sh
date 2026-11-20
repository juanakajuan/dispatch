#!/usr/bin/env bash
# Tests for media-gate.sh, against a throwaway git repo.
set -euo pipefail

HOOK="$(realpath "$(dirname "$0")/media-gate.sh")"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
fail=0

repo() { # repo DIR FILE: a clone whose HEAD adds FILE on top of origin/main
  local d="$tmp/$1"
  git init -q "$d"; git -C "$d" commit -q --allow-empty -m base
  git -C "$d" update-ref refs/remotes/origin/main HEAD
  mkdir -p "$d/$(dirname "$2")"; echo x >"$d/$2"
  git -C "$d" add -A; git -C "$d" commit -q -m change
  echo "$d"
}

# expect NAME WANT_EXIT DIR COMMAND [STDOUT_REGEX]
expect() {
  local out code=0
  out="$(jq -n --arg c "$4" --arg d "$3" '{tool_input: {command: $c}, cwd: $d}' | "$HOOK" 2>/dev/null)" || code=$?
  if [[ $code == "$2" ]] && { [[ -z ${5:-} ]] || grep -Eq "$5" <<<"$out"; }; then
    echo "ok    $1"
  else
    echo "FAIL  $1 (exit $code, want $2)"; fail=1
  fi
}

ui="$(repo ui src/ui/rows.rs)"
uitest="$(repo uitest src/ui/tests/rows.rs)"
core="$(repo core src/session/start.rs)"
printf 'Fixes it.\n\n![after](https://x/y.png)\n' >"$ui/body.md"

expect "other commands pass"                 0 "$ui" 'cargo test'
expect "a ui PR with no image is blocked"    2 "$ui" 'gh pr create --title t --body "text only"'
expect "a ui PR with an image passes"        0 "$ui" 'gh pr create --title t --body "![a](https://x/a.png)"'
expect "an image in --body-file passes"      0 "$ui" 'gh pr create --title t --body-file body.md'
expect "an image in -F passes"               0 "$ui" 'gh pr create -t t -F body.md'
expect "No images: lets a PR through"        0 "$ui" 'gh pr create --title t --body "No images: rename only"'
expect "a chained gh pr create is seen"      2 "$ui" 'git push && gh pr create --title t --body x'
expect "a PR touching only ui tests passes"  0 "$uitest" 'gh pr create --title t --body x'
expect "a PR outside ui and app passes"      0 "$core" 'gh pr create --title t --body x'
expect "gh pr view is not gated"             0 "$ui" 'gh pr view 12'
expect "an issue with no image gets a reminder" 0 "$ui" 'gh issue create --title t --body x' 'additionalContext'
expect "an issue with an image is quiet"     0 "$ui" 'gh issue create --title t --body "![a](u)"' '^$'

exit "$fail"
