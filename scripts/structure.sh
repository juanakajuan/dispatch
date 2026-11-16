#!/bin/sh
# Structure rules that clippy cannot express. Run by scripts/check.sh.
set -eu
cd "$(dirname "$0")/.."
limit=500
fail=0

for f in $(find src -name '*.rs' | sort); do
  lines=$(wc -l <"$f")
  if [ "$lines" -gt "$limit" ]; then
    echo "$f: $lines lines, over $limit. Split it into a module directory (src/<name>/mod.rs plus one file per job)."; fail=1
  fi
  if grep -nE '^[[:space:]]*// ?(-{3,}|={3,})' "$f" >/dev/null; then
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

# `WorktreePath::from_observed` canonicalizes, which is what makes every
# `WorktreePath` canonical. `proc::fake::fail` only builds a `ProcError` value.
pure="src/app src/ui src/domain.rs src/merge/plan.rs src/session/join.rs src/session/plan.rs src/session/start_argv.rs src/session/worktree.rs"
# shellcheck disable=SC2086
if grep -rnE 'crate::proc|std::process|std::fs|std::env|jobs::' $pure |
  grep -vE '^src/domain\.rs:[0-9]+: +std::fs::canonicalize\(raw\)|crate::proc::fake::fail\(' ; then
  echo "A pure module runs a process, reads a file, or reads the environment. In app, return a Cmd (Cmd::Job with a ReadJob or WriteJob) and let jobs.rs run it; in ui, take the value as a parameter, or move the code to the boundary module that already does that IO (proc, jobs, session/git, config, github)."; fail=1
fi

if grep -rnE 'Result<.*,[[:space:]]*String[[:space:]]*>|impl[[:space:]<].*From<String>' src; then
  echo "A String error loses which failure happened, so no caller can match on it. Return a typed error enum with a Display impl instead."; fail=1
fi

if grep -rnE 'Job::Read\(ReadJob::(FetchIssues|FetchPrs|ObserveSessions)' src --exclude-dir=tests; then
  echo "A poll job sent around its Poll can stack a second gh call. Send it with feed.poll.request(ReadJob::...) or state.observe.request(...)."; fail=1
fi

if grep -rnE 'toast\.text(\(\))?\.contains\(|text(\(\))?\.contains\("failed"\)' src/ui; then
  echo "The view colours a toast by its words, so a warning that says \"failed\" turns red. Match on toast.level() instead."; fail=1
fi

exit "$fail"
