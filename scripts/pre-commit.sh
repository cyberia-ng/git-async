#!/bin/sh

set -eu

if git status --porcelain | grep -q '^.[^ ]'; then
  echo "Repo is dirty" >&2
  exit 1
fi
./scripts/generate-readme.sh
git add README.md
cargo fmt --check
oxfmt --check
cargo clippy --all-features -- -D warnings
cargo test --all-features --quiet
