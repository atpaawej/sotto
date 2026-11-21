# Sotto

Hold a key, speak, and clean text lands where your cursor is. An open-source
dictation app for Linux.

This is the shell: the window, the tray icon, and the settings file. Dictation
itself is on its way.

## Run it

There are two ways to run Sotto, and they behave differently.

**Development** — Vite serves the interface on `localhost:1420`, and the debug
binary loads it from there:

```sh
npm install
npm run tauri dev
```

Do not run `src-tauri/target/debug/sotto` on its own: it is built to load the
dev server, and with Vite stopped it shows a white page that says it cannot
connect to localhost.

**Built** — the release binary carries the interface inside it and needs no
server:

```sh
npm run tauri build
src-tauri/target/release/sotto
```

## Settings

Sotto writes `~/.config/sotto/config.json` with mode 0600, because it holds API
keys. A file that cannot be parsed is reported in the window and left exactly as
it is — the app never resets your settings to recover from a bad byte.

## Desktop entry

The app id is `io.github.atpaawej.Sotto`, which Wayland portals resolve through a
`.desktop` file, and the portal only finds that file when its `Exec` points at a
binary that exists. Install it after building, or the shortcut and text
injection fail with "App info not found":

```sh
scripts/install-dev-desktop.sh --dev    # Exec points at the debug binary
scripts/install-dev-desktop.sh          # Exec points at `sotto`, or target/release
```

Injection also needs `wl-clipboard` (`wl-copy`) on `PATH`.

## Checks

```sh
npm run build                     # typecheck and bundle the interface
cd src-tauri && cargo test        # session rules, adapters, config storage
cd src-tauri && cargo clippy --all-targets -- -D warnings
```

## Licence

MIT.
