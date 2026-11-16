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
    elif [ "$lines" -lt "$ceiling" ]; then
      echo "$f: $lines lines, under its ceiling of $ceiling. Lower the ceiling in $allow to $lines."; fail=1
    fi
  elif [ "$lines" -gt "$limit" ]; then
    echo "$f: $lines lines, over $limit. Split it into a module directory (src/<name>/mod.rs plus one file per job)."; fail=1
  fi
  if grep -nE '^[[:space:]]*// ?(-{3,}|={3,})' "$f" >/dev/null && ! grep -qx "banners $f" "$allow"; then
    grep -nE '^[[:space:]]*// ?(-{3,}|={3,})' "$f" | sed "s|^|$f:|"
    echo "$f: a section banner comment. A file with sections wants to be a module: make each section its own file."; fail=1
  fi
  case "$f" in
    src/main.rs | */mod.rs) ;;
    *)
      name=$(basename "$f" .rs)
      dir=$(dirname "$f")
      if [ -f "$dir/mod.rs" ]; then parent="$dir/mod.rs"; elif [ "$dir" = src ]; then parent=src/main.rs; else parent="$dir.rs"; fi
      if ! grep -qE "^[[:space:]]*(#\[[^]]*\][[:space:]]*)?(pub(\([a-z]+\))? )?mod $name;" "$parent" 2>/dev/null; then
        echo "$f: no \`mod $name;\` in $parent, so it never compiles and its tests never run. Declare it or delete it."; fail=1
      fi
      ;;
  esac
done

if grep -rnE 'Result<.*,[[:space:]]*String[[:space:]]*>|impl[[:space:]<].*From<String>' src; then
  echo "A String error loses which failure happened, so no caller can match on it. Return a typed error enum with a Display impl instead."; fail=1
fi

if grep -rnE 'Job::Read\(ReadJob::(FetchIssues|FetchPrs|ObserveSessions)' src --exclude-dir=tests; then
  echo "A poll job sent around its Poll can stack a second gh call. Send it with feed.poll.request(ReadJob::...) or state.observe.request(...)."; fail=1
fi

if grep -rnE 'toast\.text(\(\))?\.contains\(|text(\(\))?\.contains\("failed"\)' src/ui; then
  echo "The view colours a toast by its words, so a warning that says \"failed\" turns red. Match on toast.level() instead."; fail=1
fi

awk '$1 != "banners" { print $1 }' "$allow" | while read -r f; do
  [ -f "$f" ] || { echo "$allow lists $f, which does not exist. Delete its line."; exit 1; }
done || fail=1

exit "$fail"
