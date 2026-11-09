#!/usr/bin/env bash
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
scripts/guards.sh
[[ "${1:-}" == "--fast" ]] && exit 0
cargo test --workspace
