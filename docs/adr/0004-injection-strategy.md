# Injection order: libei first, clipboard second, ydotool optional

Text reaches the focused app through one of three adapters behind the `TextInjector` seam, tried in this order: libei (via the RemoteDesktop portal), clipboard write plus a synthetic Ctrl+V, then ydotool. On X11 the same seam is satisfied by XTest.

## Why

There is no portable way to type into another app on Linux; the choice is per-compositor. libei is the protocol the ecosystem is converging on, needs no system setup, and carries any language — its cost is a permission prompt, possibly once per session. The clipboard path works everywhere and is the only sane fallback when an app ignores synthesised keys, but it still needs *a* way to send Ctrl+V, so it cannot lead. ydotool always works, but requires `input` group membership or a udev rule, a running daemon, and mangles non-ASCII text — so it is documented, never required.

## Consequences

- Injection is a user-visible capability, not a hidden detail: Settings shows which adapter is active and why, and a failed injection is a surfaced failure with a Retry (see ADR-0001).
- The clipboard adapter must save and restore the previous clipboard contents on a best-effort basis, because clobbering the user's clipboard is its main cost.
- We cannot verify that text landed. Wayland exposes no way to read another app's buffer, so v1 treats a successful inject call as success and leans on History for recovery.
- Ordering is data, not code: the adapter list is configuration, so reordering after real hardware testing is a settings change, not a rewrite.
