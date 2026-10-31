#!/usr/bin/env bash
# Single quality gate: run locally and by CI. Fails on the first failing step.
#
# The gate list mirrors docs/architecture.md §13.5 and .github/workflows/ci.yml;
# keep the three in step when a gate changes.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() { printf '\n==> %s\n' "$*"; }

if ! cargo deny --version 2>/dev/null | grep -q '0\.19'; then
  echo "error: cargo-deny 0.19.x is required (missing or wrong version)." >&2
  echo "install it with: cargo install cargo-deny --version 0.19.0 --locked" >&2
  exit 1
fi

step "cargo fmt --all --check"
cargo fmt --all --check

step "cargo clippy --workspace --all-targets --all-features -- -D warnings"
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

step "cargo test --workspace --all-features"
cargo test --locked --workspace --all-features

step "cargo test --workspace --no-default-features"
cargo test --locked --workspace --no-default-features

step "cargo doc --workspace --no-deps (warnings denied)"
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps

step "cargo deny check"
cargo deny check

printf '\nAll checks passed.\n'
