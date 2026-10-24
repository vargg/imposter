#!/usr/bin/env bash
# Every gate CI runs, in order; stops at the first failure. Run before each commit.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo llvm-cov --workspace --locked --fail-under-lines 85 --fail-under-regions 70 \
  --ignore-filename-regex '(crates/poster/src/main\.rs|crates/poster-testkit/)' --summary-only
echo "all checks passed"
