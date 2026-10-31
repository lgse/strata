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

#[test]
fn dialogs_dismiss_on_escape_alternate() {
    crate::test_support::gtk_test(
        "ui::open_with::tests::dialogs_dismiss_on_escape_alternate",
        || {
            use crate::ui::keys::{Binding, Keymap, Map, bindings::ESCAPE, use_keymap};
            use gtk::gdk::{Key, ModifierType};
            let _keymap = use_keymap(Keymap::defaults().with(
                Map::Default,
                ESCAPE,
                vec![Binding::ctrl(Key::bracketleft)],
            ));
            for (key, modifiers) in [
                (Key::bracketleft, ModifierType::CONTROL_MASK),
                (Key::Escape, ModifierType::empty()),
            ] {
                let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let overlay = gtk::Overlay::builder().child(&parent).build();
                let window = gtk::Window::builder().child(&overlay).build();
                window.present();
                show(
                    &parent,
                    vec![gio::File::for_path("/nonexistent/a.txt")],
                    vec!["text/plain".to_owned()],
                    Choices::default(),
                    OpenWithContext::Explicit,
                    Rc::new(|| {}),
                );
                let layer = overlay.last_child().expect("open-with layer");
                assert!(layer.has_css_class("app-modal-layer"));
                let controllers = layer.observe_controllers();
                let handled = (0..controllers.n_items())
                    .filter_map(|index| {
                        controllers
                            .item(index)
                            .and_downcast::<gtk::EventControllerKey>()
                    })
                    .any(|keys| {
                        keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
                    });
                assert!(handled, "{key:?}");
                assert!(crate::ui::modal::is_closing_layer(&layer), "{key:?}");
                window.destroy();
            }
        },
    );
}
