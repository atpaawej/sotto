# The Pill is built three ways, one per compositor family

The Pill — the small window that shows listening, working, done and failed — has three implementations chosen at runtime: an X11 window running under Xwayland on GNOME, a layer-shell surface on KDE/Sway/Hyprland, and a GNOME Shell extension only if the Xwayland path fails.

## Why

Wayland deliberately denies normal applications two things a floating indicator needs: absolute window placement and always-on-top. On most compositors the overlay protocol (layer-shell) solves it properly, but GNOME/Mutter does not implement layer-shell and ignores keep-above. An X11 client under Xwayland is managed by Mutter's built-in X11 window manager, which does honour positioning and above-state — so it buys the real experience for GNOME users without shipping a second codebase that must be re-tested on every GNOME release.

## Consequences

- Three implementations, one interface: `show(state)`. The rest of the app never learns which one is running.
- The Xwayland path is a spike first, not an assumption. It is verified on GNOME 50 before the Shell extension is written; if it fails, the extension is plan B and "no pill" is never a shipped answer.
- Flatpak (later) must be checked against the Xwayland path, since a sandboxed app spawning an X11 window is a different situation from a native one.
- When the Pill cannot be shown at all, failures still reach the user through a desktop notification; that is the only place the notification adapter is used.
