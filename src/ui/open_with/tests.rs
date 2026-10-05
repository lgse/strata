// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn known_players_get_a_start_option_before_their_file_arguments() {
    for (exec, expected) in [
        (
            "mpv --player-operation-mode=pseudo-gui -- %U",
            Some("mpv --player-operation-mode=pseudo-gui --start=90.500 -- %U"),
        ),
        (
            "/usr/bin/vlc --started-from-file %U",
            Some("/usr/bin/vlc --started-from-file --start-time=90.500 %U"),
        ),
        (
            "env GDK_BACKEND=x11 celluloid %U",
            Some("env GDK_BACKEND=x11 celluloid --mpv-start=90.500 %U"),
        ),
        ("mplayer %F", Some("mplayer -ss 90.500 %F")),
        (
            "/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=mpv --file-forwarding io.mpv.Mpv @@u %U @@",
            Some(
                "/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=mpv --file-forwarding io.mpv.Mpv --start=90.500 @@u %U @@",
            ),
        ),
        (
            "/usr/bin/flatpak run --file-forwarding org.videolan.VLC @@u %U @@",
            Some(
                "/usr/bin/flatpak run --file-forwarding org.videolan.VLC --start-time=90.500 @@u %U @@",
            ),
        ),
        (
            "'/opt/My Player/mpv' %U",
            Some("'/opt/My Player/mpv' --start=90.500 %U"),
        ),
        ("mpv", Some("mpv --start=90.500")),
        ("totem %U", None),
        ("/usr/bin/flatpak run org.gnome.Totem @@u %U @@", None),
        ("", None),
    ] {
        assert_eq!(command_line_at(exec, 90.5).as_deref(), expected, "{exec:?}");
    }
}
