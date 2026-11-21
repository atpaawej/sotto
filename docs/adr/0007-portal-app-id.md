# Host apps must register a portal app id before any portal call

Sotto registers itself with `org.freedesktop.host.portal.Registry.Register` on the same D-Bus connection it later uses for portals, using its Tauri identifier `io.github.atpaawej.Sotto`, and the desktop entry is installed as `io.github.atpaawej.Sotto.desktop`.

## Why

Probing on xdg-desktop-portal 1.21.1: `GlobalShortcuts.CreateSession` failed with `org.freedesktop.portal.Error.NotAllowed: An app id is required`. The old derivation of an app id from a systemd scope was removed in 1.21, so a normal, unsandboxed process now has no app id unless it says who it is. Every app-id-gated portal — GlobalShortcuts, RemoteDesktop — is unusable without it, which means no hotkey and no injection: the two features the product is made of.

Registration only succeeds when the id can be matched to a `.desktop` file of the same name. `coucou.desktop` with an app id of `fr.louisraille.coucou` fails; renaming the file fixes it. Our identifier already matches our entry name, and `scripts/install-dev-desktop.sh` puts that entry where the portal looks — but a dev build started from a shell without the entry installed gets the same `NotAllowed` error, so the script is part of the dev setup, not a packaging afterthought.

The registration is per D-Bus connection, and must happen before the first portal call. Tauri shares one GLib connection for its portal traffic, so the call belongs at startup, in the same process, on that connection.

## Consequences

- Startup does: connect to the session bus, `Register(app_id)`, and only then build the tray, bind shortcuts or open an injection session.
- A failure to register is surfaced, not ignored (ADR-0001): without an app id, dictation cannot work at all, and the window must say so with the fix — install the desktop entry.
- The registry call is idempotent in practice: a second call on an already-associated connection errors with "Connection already associated with an application ID", which is success, not a failure, and is logged as such.
- Any future Flatpak build skips this: the sandbox already provides the id.
