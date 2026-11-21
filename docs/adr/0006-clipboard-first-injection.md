# Injection goes through the clipboard, because keysym typing only types what the keymap has

The `TextInjector` order changes: the clipboard path leads, and keysym typing through the RemoteDesktop portal becomes a fallback rather than the default.

## Why

Probed on this machine on 2026-11-21 — GNOME 50.1, xdg-desktop-portal 1.21.1, xdg-desktop-portal-gnome 50.0, one GTK4 entry field, `NotifyKeyboardKeysym` per character:

| typed | landed |
| --- | --- |
| `Hello world ABC 123` | `Hello world ABC 123` |
| `Café Grüße — 日本語 123` | `Caf Gre   123` |

Every character the active keymap has no key for is dropped, silently, with no error from the portal. `é`, `ü`, `ß`, the em dash and all CJK are gone. This falsifies ADR-0004's claim that libei "carries any language": the transport carries keysyms, and Mutter only turns a keysym into keystrokes when the layout can produce it.

A dictation app that cannot type an accented name or a Japanese sentence is broken for most of the world, so this cannot be the leading path.

## Consequences

- Clipboard write plus a synthetic Ctrl+V becomes the primary adapter; it is the only mechanism that carries arbitrary Unicode, and the Ctrl+V keystroke itself is ASCII and provably works.
- Keysym typing stays as a fallback for environments where clipboard paste is unavailable, and it must check the text: if a character has no keymap entry, do not claim success.
- The clipboard write mechanism on Wayland is the next thing to verify, not an assumption: `org.freedesktop.portal.Clipboard` inside an existing RemoteDesktop session, versus `wl-clipboard`. Whoever implements injection confirms which works from an app that does not hold keyboard focus.
- ydotool's non-ASCII behaviour needs no further investigation — it is documented as never required, and the same keymap limitation applies to it.
- ADR-0004's ordering is amended, not deleted: the seam, the adapters and the "ordering is data" consequence all stand.
