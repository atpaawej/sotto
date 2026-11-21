fn main() {
    // The pill has to place itself at the bottom of the screen and stay above
    // other windows, which Wayland does not allow but Xwayland does (ADR-0005).
    // Only GNOME needs this; other compositors get their own surface later.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && is_gnome() {
        std::env::set_var("GDK_BACKEND", "x11");
    }

    sotto_lib::run()
}

fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|desktop| desktop.to_lowercase().contains("gnome"))
        .unwrap_or(false)
}
