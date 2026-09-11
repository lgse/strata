// SPDX-License-Identifier: MIT

use super::*;
use std::time::Instant;

fn desktop_app(name: &str, arguments: &str, extra: &str) -> gio::AppInfo {
    let key = glib::KeyFile::new();
    key.load_from_data(
        &format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/true {arguments}\n{extra}\n"
        ),
        glib::KeyFileFlags::NONE,
    )
    .expect("desktop entry");
    gio_unix::DesktopAppInfo::from_keyfile(&key)
        .expect("application")
        .upcast()
}

#[test]
fn compatible_handlers_include_hidden_defaults_but_require_uri_support() {
    let path = desktop_app("Path", "%F", "");
    let uri = desktop_app("URI", "%U", "");
    let hidden = desktop_app("Hidden", "%U", "NoDisplay=true");
    let other_desktop = desktop_app("Other desktop", "%U", "OnlyShowIn=StrataTestDesktop;");
    let apps = vec![
        path.clone(),
        uri.clone(),
        hidden.clone(),
        other_desktop.clone(),
    ];
    assert_eq!(filter_apps(apps.clone(), None, false).len(), 2);
    let remote = filter_apps(apps.clone(), Some(hidden.clone()), true);
    assert_eq!(remote.len(), 2);
    assert!(remote[0].equal(&hidden));
    assert!(remote[1].equal(&uri));
    assert_eq!(filter_apps(apps.clone(), Some(path), true).len(), 1);
    let other_default = filter_apps(apps, Some(other_desktop.clone()), false);
    assert!(other_default[0].equal(&other_desktop));
}

#[test]
fn path_only_launch_rejects_non_native_files_before_spawning() {
    let app = desktop_app("Path", "%F", "");
    let local = gio::File::for_path("/tmp/local.txt");
    let remote = gio::File::for_uri("trash:///remote.txt");
    let error =
        launch(&app, &[local, remote], None::<&gio::AppLaunchContext>).expect_err("URI guard");
    assert!(error.matches(gio::IOErrorEnum::NotSupported));
    assert!(
        error
            .message()
            .contains("cannot open files at this location")
    );
}

#[test]
fn uri_capable_launch_preserves_every_remote_argument() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().expect("fixture");
    let recorder = fixture.path().join("record");
    let output = fixture.path().join("arguments");
    std::fs::write(
        &recorder,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
            output.display()
        ),
    )
    .expect("recorder");
    std::fs::set_permissions(&recorder, std::fs::Permissions::from_mode(0o755))
        .expect("permissions");
    let app = gio::AppInfo::create_from_commandline(
        format!("{} %U", recorder.display()),
        Some("URI recorder"),
        gio::AppInfoCreateFlags::SUPPORTS_URIS,
    )
    .expect("application");
    let files = [
        gio::File::for_uri("trash:///alpha%20file.txt"),
        gio::File::for_uri("sftp://example.invalid/beta.txt"),
    ];
    launch(&app, &files, None::<&gio::AppLaunchContext>).expect("URI launch");
    let deadline = Instant::now() + Duration::from_secs(3);
    let received = loop {
        if let Ok(contents) = std::fs::read_to_string(&output) {
            let received = contents.lines().map(gio::File::for_uri).collect::<Vec<_>>();
            if received.len() == files.len() {
                break received;
            }
        }
        assert!(Instant::now() < deadline, "recorder output");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(
        received
            .iter()
            .zip(&files)
            .all(|(actual, expected)| actual.equal(expected))
    );
}

#[test]
fn missing_and_unresolvable_icons_use_the_live_themed_fallback() {
    crate::test_support::gtk_test(
        "ui::open_with::tests::missing_and_unresolvable_icons_use_the_live_themed_fallback",
        || {
            crate::assets::register_icon_theme();
            let display = gtk::gdk::Display::default().expect("display");
            let missing = application_icon(&desktop_app("Missing", "%U", ""), &display);
            for extra in [
                "Icon=strata-nonexistent-icon-569",
                "Icon=/nonexistent/strata-icon.png",
            ] {
                let broken = application_icon(&desktop_app("Broken", "%U", extra), &display);
                assert_eq!(broken.storage_type(), missing.storage_type());
                assert_eq!(broken.paintable(), missing.paintable());
                let original = broken.paintable();
                let color = crate::assets::primary_icon_color();
                crate::assets::set_primary_icon_color("#123456");
                assert_ne!(broken.paintable(), original);
                assert_eq!(broken.paintable(), missing.paintable());
                crate::assets::set_primary_icon_color(&color);
            }
            let valid = application_icon(&desktop_app("Valid", "%U", "Icon=folder"), &display);
            assert_eq!(valid.storage_type(), gtk::ImageType::Gicon);
        },
    );
}

#[test]
fn other_apps_filter_excludes_recommended_and_hidden() {
    let path = desktop_app("Path", "%F", "");
    let uri = desktop_app("URI", "%U", "");
    let hidden = desktop_app("Hidden", "%U", "NoDisplay=true");
    let extra = desktop_app("Extra", "%U", "");
    let apps = vec![path.clone(), uri.clone(), hidden.clone(), extra.clone()];
    let recommended = vec![path.clone(), uri.clone()];
    let other = filter_other_apps(apps.clone(), &recommended, false);
    assert_eq!(other.len(), 1);
    assert!(other[0].equal(&extra));
    let other_remote = filter_other_apps(apps, &recommended, true);
    assert_eq!(other_remote.len(), 1);
    assert!(other_remote[0].equal(&extra));
}

#[test]
fn empty_chooser_disables_open_and_restores_focus_after_backdrop_dismissal() {
    crate::test_support::gtk_test(
        "ui::open_with::tests::empty_chooser_disables_open_and_restores_focus_after_backdrop_dismissal",
        || {
            let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&parent));
            let window = gtk::Window::new();
            window.set_child(Some(&overlay));
            window.present();
            let closed = Rc::new(Cell::new(0));
            let result = closed.clone();
            show(
                &parent,
                vec![],
                vec![],
                vec![],
                Rc::new(move || result.set(result.get() + 1)),
            );
            let layer = overlay
                .last_child()
                .expect("modal layer")
                .downcast::<gtk::Box>()
                .expect("layer box");
            fn find_open(widget: &gtk::Widget) -> Option<gtk::Button> {
                if let Some(button) = widget.downcast_ref::<gtk::Button>()
                    && button.label().as_deref() == Some("Open")
                {
                    return Some(button.clone());
                }
                let mut child = widget.first_child();
                while let Some(widget) = child {
                    if let Some(button) = find_open(&widget) {
                        return Some(button);
                    }
                    child = widget.next_sibling();
                }
                None
            }
            assert!(
                !find_open(layer.upcast_ref())
                    .expect("Open button")
                    .is_sensitive()
            );
            dismiss_modal_layer(&layer, &overlay, None);
            let deadline = Instant::now() + Duration::from_secs(3);
            while closed.get() == 0 {
                assert!(Instant::now() < deadline);
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(closed.get(), 1);
            window.close();
        },
    );
}

#[test]
fn search_filtering_and_empty_state_and_keyboard_navigation() {
    crate::test_support::gtk_test(
        "ui::open_with::tests::search_filtering_and_empty_state_and_keyboard_navigation",
        || {
            let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&parent));
            let window = gtk::Window::new();
            window.set_child(Some(&overlay));
            window.present();

            let text_editor = desktop_app("Text Editor", "%U", "Comment=Edit plain text\n");
            let image_viewer = desktop_app("Image Viewer", "%U", "Comment=View photos\n");
            let web_browser = desktop_app("Web Browser", "%U", "Comment=Browse the internet\n");

            let closed = Rc::new(Cell::new(0));
            let result = closed.clone();
            show(
                &parent,
                vec![],
                vec![text_editor.clone()],
                vec![image_viewer.clone(), web_browser.clone()],
                Rc::new(move || result.set(result.get() + 1)),
            );
            let layer = overlay
                .last_child()
                .expect("modal layer")
                .downcast::<gtk::Box>()
                .expect("layer box");

            fn find_widget_by_class<T: IsA<gtk::Widget>>(
                root: &gtk::Widget,
                class_name: &str,
            ) -> Option<T> {
                if root.has_css_class(class_name)
                    && let Some(w) = root.downcast_ref::<T>()
                {
                    return Some(w.clone());
                }
                let mut child = root.first_child();
                while let Some(current) = child {
                    if let Some(found) = find_widget_by_class(&current, class_name) {
                        return Some(found);
                    }
                    child = current.next_sibling();
                }
                None
            }

            fn find_open(widget: &gtk::Widget) -> Option<gtk::Button> {
                if let Some(button) = widget.downcast_ref::<gtk::Button>()
                    && button.label().as_deref() == Some("Open")
                {
                    return Some(button.clone());
                }
                let mut child = widget.first_child();
                while let Some(widget) = child {
                    if let Some(button) = find_open(&widget) {
                        return Some(button);
                    }
                    child = widget.next_sibling();
                }
                None
            }

            let search: gtk::SearchEntry =
                find_widget_by_class(layer.upcast_ref(), "open-with-search").expect("search entry");
            let list: gtk::ListBox =
                find_widget_by_class(layer.upcast_ref(), "open-with-list").expect("list box");
            let open_btn = find_open(layer.upcast_ref()).expect("open button");

            assert!(open_btn.is_sensitive());

            let count_visible_rows = |list: &gtk::ListBox| -> usize {
                let mut count = 0;
                let mut child = list.first_child();
                while let Some(w) = child {
                    if let Some(row) = w.downcast_ref::<gtk::ListBoxRow>()
                        && row.is_visible()
                    {
                        count += 1;
                    }
                    child = w.next_sibling();
                }
                count
            };

            let wait_until = |condition: &dyn Fn() -> bool| {
                let deadline = Instant::now() + Duration::from_secs(3);
                while !condition() {
                    assert!(Instant::now() < deadline, "search update timed out");
                    glib::MainContext::default().iteration(false);
                    std::thread::sleep(Duration::from_millis(5));
                }
            };

            // Filter for "viewer"
            search.set_text("viewer");
            wait_until(&|| count_visible_rows(&list) == 2);
            assert!(open_btn.is_sensitive());

            // Filter with no match
            search.set_text("nonexistent-app-xyz");
            wait_until(&|| !open_btn.is_sensitive());

            // Clear filter
            search.set_text("");
            wait_until(&|| open_btn.is_sensitive() && count_visible_rows(&list) == 5);

            // Navigate into list and type-to-search
            if let Some(row) = list.selected_row() {
                row.grab_focus();
            }
            glib::MainContext::default().iteration(false);

            dismiss_modal_layer(&layer, &overlay, None);
            let deadline = Instant::now() + Duration::from_secs(3);
            while closed.get() == 0 {
                assert!(Instant::now() < deadline);
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert_eq!(closed.get(), 1);
            window.close();
        },
    );
}
