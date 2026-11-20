#!/usr/bin/env bash
# Claude Code PreToolUse hook (Bash): a PR that changes what a user sees must
# carry an image. AGENTS.md, "Issues and PRs", gives the rule.
#
# - `gh pr create` whose diff against origin/main touches src/ui/ or src/app/
#   (not their tests/) is blocked when the body has no Markdown image.
# - `gh issue create` with no image gets a reminder, not a block: a script
#   cannot tell whether a bug is visible.
# - A body line `No images: <reason>` lets a PR through.
#
# Input is the hook JSON on stdin. Exit 2 blocks the call and shows stderr to Claude.
set -euo pipefail

input="$(cat)"
cmd="$(jq -r '.tool_input.command // ""' <<<"$input")"
cwd="$(jq -r '.cwd // "."' <<<"$input")"

kind=""
if grep -Eq '(^|[^[:alnum:]_-])gh[[:space:]]+pr[[:space:]]+create([[:space:]]|$)' <<<"$cmd"; then kind=pr
elif grep -Eq '(^|[^[:alnum:]_-])gh[[:space:]]+issue[[:space:]]+create([[:space:]]|$)' <<<"$cmd"; then kind=issue
else exit 0
fi

# The body is in the command (--body/-b) or in a file (--body-file/-F).
body="$cmd"
file="$(grep -Eo -- '(--body-file|-F)[ =]+[^ ;&|]+' <<<"$cmd" | head -1 | sed -E 's/^(--body-file|-F)[ =]+//; s/^["'\'']//; s/["'\'']$//')" || true
if [[ -n $file ]]; then
  [[ $file == /* ]] || file="$cwd/$file"
  [[ -f $file ]] && body+=$'\n'"$(cat "$file")"
fi

has_image() { grep -Eq '!\[[^]]*\]\([^)]+\)' <<<"$body"; }
opted_out() { grep -Eiq 'no images:[[:space:]]*[^[:space:]]' <<<"$body"; }

has_image && exit 0
opted_out && exit 0

how='Make images with .claude/skills/verify/scripts/media.sh (png or gif on verify shots), run `media.sh publish <kind>-<n> FILE...`, and paste the lines it prints into the body. See "Evidence" in .claude/skills/verify/SKILL.md.'

if [[ $kind == issue ]]; then
  jq -n --arg c "Reminder (AGENTS.md, Issues and PRs): if this issue reports a visible bug, add a screenshot or GIF. $how" \
    '{hookSpecificOutput: {hookEventName: "PreToolUse", additionalContext: $c}}'
  exit 0
fi

visible="$(git -C "$cwd" diff --name-only origin/main...HEAD 2>/dev/null |
  grep -E '^src/(ui|app)/' | grep -v '/tests/' || true)"
[[ -z $visible ]] && exit 0

{
  echo "Blocked: this PR changes what a user sees, but its body has no image (AGENTS.md, Issues and PRs)."
  echo "Files:"; sed 's/^/  /' <<<"$visible"
  echo "$how"
  echo 'If nothing visible changed, add a body line "No images: <reason>".'
} >&2
exit 2
