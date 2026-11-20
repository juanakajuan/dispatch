#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
scripts/structure.sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
.claude/skills/verify/scripts/verify_test.sh
.claude/hooks/media-gate_test.sh
