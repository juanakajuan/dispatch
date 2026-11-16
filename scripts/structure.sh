#!/bin/sh
# Structure rules that clippy cannot express. Run by scripts/check.sh.
set -eu
cd "$(dirname "$0")/.."
limit=500
allow=scripts/structure-allow.txt
fail=0

for f in $(find src -name '*.rs' | sort); do
  lines=$(wc -l <"$f")
  ceiling=$(awk -v f="$f" '$1 == f { print $2 }' "$allow")
  if [ -n "$ceiling" ]; then
    if [ "$lines" -le "$limit" ]; then
      echo "$f: $lines lines, now under $limit. Delete its line from $allow."; fail=1
    elif [ "$lines" -gt "$ceiling" ]; then
      echo "$f: $lines lines, over its ceiling of $ceiling in $allow. Split it into a module directory; do not raise the ceiling."; fail=1
    fi
  elif [ "$lines" -gt "$limit" ]; then
    echo "$f: $lines lines, over $limit. Split it into a module directory (src/<name>/mod.rs plus one file per job)."; fail=1
  fi
  if grep -nE '^[[:space:]]*// ?(-{3,}|={3,})' "$f" >/dev/null && ! grep -qx "banners $f" "$allow"; then
    grep -nE '^[[:space:]]*// ?(-{3,}|={3,})' "$f" | sed "s|^|$f:|"
    echo "$f: a section banner comment. A file with sections wants to be a module: make each section its own file."; fail=1
  fi
done

awk '$1 != "banners" { print $1 }' "$allow" | while read -r f; do
  [ -f "$f" ] || { echo "$allow lists $f, which does not exist. Delete its line."; exit 1; }
done || fail=1

exit "$fail"
