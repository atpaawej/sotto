#!/usr/bin/env bash
# Install or update Sotto from the latest GitHub release.
#
# The app ships as a `.deb`, so installing or updating it needs sudo:
#
#   curl -fsSL https://raw.githubusercontent.com/atpaawej/typeless/master/scripts/install-sotto.sh | sudo bash
#
# The `.deb` is downloaded from the release, its SHA-256 is verified against
# the checksum the release ships next to it, and dpkg installs it. The
# `.desktop` entry and icon come with the package.

set -euo pipefail

repo="atpaawej/typeless"

# The release's download URLs: the .deb first, then the checksum beside it.
assets=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" |
  grep -o '"browser_download_url": *"[^"]*"' |
  sed 's/"browser_download_url": *"//; s/"$//' || true)

deb_url=$(printf '%s\n' "$assets" | grep -m1 'amd64.*\.deb$' || true)
if [ -z "$deb_url" ]; then
  deb_url=$(printf '%s\n' "$assets" | grep -m1 '\.deb$' || true)
fi
if [ -z "$deb_url" ]; then
  echo "no .deb found in the latest release of $repo" >&2
  exit 1
fi

sha_name="$(basename "$deb_url").sha256"
sha_url=$(printf '%s\n' "$assets" | grep -F -m1 "$sha_name" || true)

run() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  else
    sudo "$@"
  fi
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

name="$(basename "$deb_url")"
curl -fsSL -o "$work/$name" "$deb_url"
if [ -n "$sha_url" ]; then
  curl -fsSL -o "$work/$sha_name" "$sha_url"
  (cd "$work" && sha256sum -c "$sha_name")
else
  echo "No checksum was published with $name; installing without verification." >&2
fi

run dpkg -i "$work/$name"
if command -v update-desktop-database >/dev/null; then
  run update-desktop-database /usr/share/applications
fi

echo "Installed $name."
echo "Sotto needs a restart to pick up the new version; open it from your menu."
