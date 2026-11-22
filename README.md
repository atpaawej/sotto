# Sotto

Hold a key, speak, and clean text lands where your cursor is. Open-source desktop dictation for Linux, powered by your own Groq or ElevenLabs API key.

## Install on Ubuntu / Debian

Run one command to install or update Sotto:

```sh
curl -fsSL https://raw.githubusercontent.com/atpaawej/typeless/master/scripts/install-sotto.sh | sudo bash
```

The installer downloads the latest `.deb` from [GitHub Releases](https://github.com/atpaawej/typeless/releases/latest), verifies its SHA-256 checksum against the `.sha256` file published beside it, and installs `sotto`, the `.desktop` entry, and the app icon with `dpkg`. You can also download the `.deb` or `.AppImage` manually from the releases page.

## Why Sotto

Typing long messages, code comments, and prompts by hand slows you down, and most Linux dictation tools either break on Wayland or lock you into slow local models.

With Sotto, you stay in whatever app is focused. Hold your shortcut key in a terminal, editor, or browser, talk naturally, and release. Sotto records your voice, transcribes it on Groq's Whisper `large-v3-turbo`, cleans up filler words, punctuation, and mid-sentence self-corrections, and injects the finished text straight at your cursor in ~400 ms.

## What you get

- **Bring your own API key:** Choose Groq (default, fast, generous free tier) or ElevenLabs Scribe. Your keys stay in `~/.config/sotto/config.json` with `0600` permissions, and audio is sent only to your active Provider.
- **Smart Cleanup and Dictionary:** Pick `none`, `light` (default), or `medium` cleanup to strip stutters and fix casing while keeping your phrasing. Add project names, acronyms, and custom spellings to your Dictionary so they land right the first time.
- **Built for Wayland and X11:** Text Injection leads with `wl-clipboard` (`wl-copy`) plus `Ctrl+V` while restoring your previous clipboard on a best-effort basis, with `ydotool` available as an optional fallback.
- **History and one-click Retry:** Every Dictation and its Recording stay on disk. If the network drops or you want to compare providers, retry the stored Recording without speaking again.
- **Clear failures:** If a key is invalid, a mic is unavailable, or injection fails, the Pill tells you why immediately and Settings keeps the full detail.
- **In-app update check:** Settings → About checks for newer releases and lets you install updates without hunting for links.

## First run

Launching Sotto for the first time opens a four-step setup:

1. Pick Groq or ElevenLabs.
2. Paste your API key and run a live test request to verify it.
3. Choose your hold-to-dictate shortcut (double-tap locks dictation hands-free).
4. Dictate one sentence into any app. Setup closes once the text lands.

## Development

**Run in dev mode** (Vite serves the UI on `localhost:1420`):

```sh
npm install
npm run tauri dev
```

Do not run `src-tauri/target/debug/sotto` directly without Vite running, as debug builds load the UI from the dev server.

**Wayland desktop entry for local builds:**

Wayland portals resolve the app ID `io.github.atpaawej.Sotto` through a `.desktop` file pointing to an existing binary. Install the dev desktop entry before testing shortcuts or injection from a local build:

```sh
scripts/install-dev-desktop.sh --dev    # points Exec at target/debug/sotto
scripts/install-dev-desktop.sh          # points Exec at sotto or target/release/sotto
```

**Build and checks:**

```sh
npm run build                                        # typecheck and bundle frontend
npm run tauri build                                  # build .deb and AppImage
cd src-tauri && cargo test                           # unit and integration tests
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

## Licence

MIT.
