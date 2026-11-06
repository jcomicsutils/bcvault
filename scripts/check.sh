#!/usr/bin/env bash
set -euo pipefail

cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
if rg -n 'corner_radius\(' src/ui 2>/dev/null | rg -v 'CornerRadius::ZERO'; then
    echo "ERROR: non-zero corner_radius found in src/ui" >&2
    exit 1
fi
