#!/usr/bin/env bash
# Installs the newest already-compiled bcvault-fx binary and refreshes its .desktop entry.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

bin=""
for c in target/release/bcvault-fx target/debug/bcvault-fx; do
  [[ -x "$c" ]] || continue
  if [[ -z "$bin" || "$c" -nt "$bin" ]]; then bin="$c"; fi
done
[[ -n "$bin" ]] || { echo 'install-fx: no compiled bcvault-fx found, build it first' >&2; exit 1; }

bindir="$HOME/.local/bin"
appdir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$bindir" "$appdir"
install -m 755 "$bin" "$bindir/bcvault-fx"
cat > "$appdir/bcvault-fx.desktop" <<DESK
[Desktop Entry]
Type=Application
Name=bcvault
Comment=Bandcamp collection vault
Exec=$bindir/bcvault-fx
Icon=audio-x-generic
Terminal=false
Categories=AudioVideo;Audio;Music;
DESK
command -v update-desktop-database >/dev/null && update-desktop-database "$appdir" || true
echo "install-fx: installed $bin -> $bindir/bcvault-fx"
