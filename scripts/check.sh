#!/usr/bin/env bash
set -euo pipefail

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
if rg -n 'corner_radius\(' crates/bcvault/src/ui 2>/dev/null | rg -v 'CornerRadius::ZERO'; then
    echo "ERROR: non-zero corner_radius found in crates/bcvault/src/ui" >&2
    exit 1
fi
if [ -d crates/bcvault-fx/src ]; then
    if rg -n 'circle_|Shape::circle|rounding|corner_radius|CornerRadius::(same|from|new)|Spinner' crates/bcvault-fx/src 2>/dev/null | rg -v 'CornerRadius::ZERO'; then
        echo "ERROR: banned round primitive or Spinner found in crates/bcvault-fx/src" >&2
        exit 1
    fi
fi
