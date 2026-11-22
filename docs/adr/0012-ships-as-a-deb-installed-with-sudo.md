# Ships as a `.deb`, installed with sudo; updates replace it over the network

Part of #1. Implements the packaging decision.

## Why

The app now survives the machines that are not this one, so it installs like
an ordinary desktop app. The user decided on a `.deb`-based installer, so
installing needs sudo: a `.deb` can change the system binary and desktop
entry, and there is no sudo-free curl installer — the curl path is
`scripts/install-sotto.sh | sudo bash`.

A tag builds both an AppImage and a `.deb` (`npm run tauri build`); the
`.deb` is the installer, and the AppImage remains a manual download.

The update check reports newer releases without installing them, because a
running process is not a moment to swap files. Installing is one click —
Settings → About → Install update — or one command:

```sh
curl -fsSL https://raw.githubusercontent.com/atpaawej/typeless/main/scripts/install-sotto.sh | sudo bash
```

## Consequences

- Every install or update needs sudo, and there is no in-place swap: dpkg
  replaces the binary under the running app, so the user restarts Sotto to
  pick the new version up.
- The installer verifies the release's SHA-256: the tag workflow publishes
  `<deb>.sha256` beside each `.deb`, and both the script and the Settings
  install path refuse a checksum mismatch.
- The desktop entry and icon ship in the package
  (`bundle.linux.deb.files` in `tauri.conf.json`), so a fresh install
  resolves the portal app id (ADR-0007) without a dev script.
- The README's install section is one command; first-run instructions are
  unchanged, so a clean machine installs and dictates from the README alone.
- AppImage needs `libfuse2`; the `.deb` does not, and it is the installer.
