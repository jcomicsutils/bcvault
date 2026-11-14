#!/usr/bin/env bash
# Every guard command must print nothing. Add new guards here, never weaken one.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
fail=0

guard() { # guard <description> <command...>
  local desc="$1"; shift
  local out; out="$("$@" 2>/dev/null || true)"
  if [[ -n "$out" ]]; then printf 'GUARD FAILED: %s\n%s\n\n' "$desc" "$out" >&2; fail=1; fi
}
no_round() { rg -n "$1" "${@:2}" | rg -v 'CornerRadius::ZERO'; }

UI_DIRS=(crates/bcvault/src/ui crates/bcvault-fx/src/views crates/bcvault-fx/src/overlays
         crates/bcvault-fx/src/paint crates/bcvault-fx/src/motion crates/bcvault-fx/src/theme)

guard "UI code must not block or do I/O" \
  rg -n 'block_on|rusqlite|reqwest|std::fs|tokio::fs|rfd::FileDialog' "${UI_DIRS[@]}" --glob '!**/theme/mod.rs' --glob '!**/theme/registry.rs'
guard "classic UI: non-zero corner radius" no_round 'corner_radius\(' crates/bcvault/src/ui
guard "fx: round primitives" no_round \
  'circle_|Shape::circle|rounding|corner_radius|CornerRadius::(same|from|new)|Spinner' crates/bcvault-fx/src
guard "reqwest clients must be built in bandcamp module only" \
  rg -n 'reqwest::(Client|ClientBuilder)' crates --glob '!**/bandcamp/**' --glob '!**/tests/**' --glob '!**/examples/**'
guard "secrets in log macros" \
  rg -n -i '(info|warn|error|debug|trace)!\(.*(password|passwd|secret|[?&]sig=|\bsig\b)' crates
guard "shipped migrations must not be modified" bash -c '
  if git rev-parse --verify -q origin/main >/dev/null; then
    git diff --name-status origin/main...HEAD -- "crates/*/migrations/*.sql" | grep -v "^A" || true
  fi'

exit "$fail"
