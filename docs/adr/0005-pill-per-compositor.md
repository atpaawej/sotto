# The Pill is built three ways, one per compositor family

The Pill — the small window that shows listening, working, done and failed — has three implementations chosen at runtime: an X11 window running under Xwayland on GNOME, a layer-shell surface on KDE/Sway/Hyprland, and a GNOME Shell extension only if the Xwayland path fails.

## Why

Wayland deliberately denies normal applications two things a floating indicator needs: absolute window placement and always-on-top. On most compositors the overlay protocol (layer-shell) solves it properly, but GNOME/Mutter does not implement layer-shell and ignores keep-above. An X11 client under Xwayland is managed by Mutter's built-in X11 window manager, which does honour positioning and above-state — so it buys the real experience for GNOME users without shipping a second codebase that must be re-tested on every GNOME release.

## Consequences

- Three implementations, one interface: `show(state)`. The rest of the app never learns which one is running.
- The Xwayland path is a spike first, not an assumption. Verified on GNOME 50.1 on 2026-11-21 (ticket #3): a GTK window under Xwayland landed at exactly the requested coordinates on a 1600x900 screen (asked 640,760, got 640,760), Mutter accepted `_NET_WM_STATE_ABOVE` alongside skip-taskbar and utility type, and the window stayed visible above other apps without taking focus. The Shell extension is not needed for v1.
- Flatpak (later) must be checked against the Xwayland path, since a sandboxed app spawning an X11 window is a different situation from a native one.
- When the Pill cannot be shown at all, failures still reach the user through a desktop notification; that is the only place the notification adapter is used.
