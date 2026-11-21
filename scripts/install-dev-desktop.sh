#!/usr/bin/env bash
# Install the desktop entry and icon for a development build, so the app has a
# resolvable portal app id (io.github.atpaawej.Sotto) instead of a bare process.
#
#   scripts/install-dev-desktop.sh            # Exec=sotto, for an installed build
#   scripts/install-dev-desktop.sh --dev      # Exec points at target/debug/sotto

set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}

# The portal resolves an app id through the desktop file's Exec: an entry whose
# command cannot be run is invisible to it, and registration fails with "App
# info not found". So the path is always absolute and always exists.
if [[ "${1:-}" == "--dev" ]]; then
  exec_line="$root/src-tauri/target/debug/sotto"
else
  exec_line="$(command -v sotto || true)"
  if [[ -z "$exec_line" ]]; then
    exec_line="$root/src-tauri/target/release/sotto"
  fi
fi

if [[ ! -x "$exec_line" ]]; then
  echo "no runnable binary at $exec_line; build it before installing the entry" >&2
  exit 1
fi

applications="$data/applications"
icons="$data/icons/hicolor/128x128/apps"

mkdir -p "$applications" "$icons"
install -m 644 "$root/packaging/io.github.atpaawej.Sotto.desktop" "$applications/io.github.atpaawej.Sotto.desktop"
install -m 644 "$root/src-tauri/icons/128x128.png" "$icons/io.github.atpaawej.Sotto.png"

if [[ "$exec_line" != "sotto" ]]; then
  sed -i "s|^Exec=.*|Exec=$exec_line|" "$applications/io.github.atpaawej.Sotto.desktop"
fi

if command -v update-desktop-database >/dev/null; then
  update-desktop-database "$applications"
else
  echo "update-desktop-database is not installed; the app grid cache was not refreshed"
fi

echo "Installed $applications/io.github.atpaawej.Sotto.desktop (Exec=$exec_line)"
