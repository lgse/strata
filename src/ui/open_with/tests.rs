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

fn path_only_app() -> gio::AppInfo {
    let app = gio::AppInfo::create_from_commandline(
        "/bin/true %F",
        Some("Path handler"),
        gio::AppInfoCreateFlags::NONE,
    )
    .expect("application");
    assert!(!app.supports_uris());
    app
}

fn mounted_non_native_file_with_path() -> Option<gio::File> {
    gio::VolumeMonitor::get()
        .mounts()
        .into_iter()
        .map(|mount| mount.root())
        .find(|file| !file.is_native() && file.path().is_some())
}

// Drain GIO callbacks under the shared context lock to prevent cross-test thread-affinity failures.
fn launch_and_settle_recent(
    app: &gio::AppInfo,
    files: &[gio::File],
    expected: usize,
    accept: bool,
) -> (Result<(), glib::Error>, Vec<String>) {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    let recorded = Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = recorded.clone();
    let result =
        launch_with_recent_registration(app, files, None::<&gio::AppLaunchContext>, move |file| {
            sink.borrow_mut().push(file.uri().to_string());
            accept
        });
    let deadline = Instant::now() + Duration::from_secs(5);
    while recorded.borrow().len() < expected && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    while context.pending() {
        context.iteration(false);
    }
    let recorded = recorded.borrow().clone();
    (result, recorded)
}

#[test]
fn successful_launch_registers_each_file_uri_after_launch() {
    let files = [
        gio::File::for_path("/tmp/ticket-05-first.txt"),
        gio::File::for_path("/tmp/ticket-05-second.txt"),
    ];
    let app = path_only_app();

    let (result, registered) = launch_and_settle_recent(&app, &files, files.len(), true);

    assert!(result.is_ok());
    assert_eq!(
        registered,
        files
            .iter()
            .map(|file| file.uri().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn launch_returns_before_recent_registration_touches_the_filesystem() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    let files = [gio::File::for_path("/tmp/ticket-05-deferred.txt")];
    let registered = Rc::new(Cell::new(false));
    let sink = registered.clone();
    let app = path_only_app();

    launch_with_recent_registration(&app, &files, None::<&gio::AppLaunchContext>, move |_| {
        sink.set(true);
        true
    })
    .expect("path launch");

    assert!(
        !registered.get(),
        "launch must not wait for the Recent type query"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !registered.get() && Instant::now() < deadline {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(registered.get(), "the queued registration should still run");
}

#[test]
fn failed_application_launch_does_not_register_recent_files() {
    let files = [gio::File::for_path("/tmp/ticket-05-failed.txt")];
    let app = gio::AppInfo::create_from_commandline(
        "/definitely/not-a-real-strata-launcher %F",
        Some("Failing application"),
        gio::AppInfoCreateFlags::NONE,
    )
    .expect("application");

    let (result, registered) = launch_and_settle_recent(&app, &files, 0, false);

    assert!(result.is_err());
    assert!(registered.is_empty());
}

#[test]
fn recent_registration_failure_does_not_fail_successful_launch() {
    let files = [gio::File::for_path(
        "/tmp/ticket-05-registration-failure.txt",
    )];
    let app = path_only_app();

    let (result, registered) = launch_and_settle_recent(&app, &files, 1, false);

    assert!(result.is_ok());
    assert_eq!(registered.len(), 1);
}

#[test]
fn successful_open_with_does_not_register_a_directory() {
    let fixture = tempfile::tempdir().expect("fixture");
    let target_path = fixture.path().join("target.txt");
    std::fs::write(&target_path, "target").expect("target");
    let directory = gio::File::for_path(fixture.path());
    let target = gio::File::for_path(&target_path);
    let files = [directory, target.clone()];
    let app = path_only_app();

    let (result, registered) = launch_and_settle_recent(&app, &files, 1, true);

    result.expect("Open With launch");
    assert_eq!(registered, vec![target.uri().to_string()]);
}

#[test]
fn recent_open_registers_the_target_uri_instead_of_the_collection_uri() {
    let target = gio::File::for_path("/tmp/ticket-05-real-target.txt");
    let recent_entry = gio::File::for_uri("recent:///ticket-05-entry");
    let files = [recent_entry, target.clone()];
    let app = gio::AppInfo::create_from_commandline(
        "/bin/true %U",
        Some("URI handler"),
        gio::AppInfoCreateFlags::SUPPORTS_URIS,
    )
    .expect("application");

    let (result, registered) = launch_and_settle_recent(&app, &files, 1, true);

    result.expect("Open With launch");
    assert_eq!(registered, vec![target.uri().to_string()]);
}

fn assert_requires_uri_handlers(file: &gio::File) {
    assert!(!file.is_native());
    assert!(
        file.path().is_none(),
        "{} should have no local path",
        file.uri()
    );
    assert!(requires_uri_handlers(std::slice::from_ref(file)));
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
fn uri_handlers_are_required_only_without_a_local_path() {
    let native = gio::File::for_path("/tmp/notes.txt");
    assert!(native.is_native());
    assert!(native.path().is_some());
    assert!(!requires_uri_handlers(std::slice::from_ref(&native)));

    let trash = gio::File::for_uri("trash:///notes.txt");
    assert_requires_uri_handlers(&trash);

    for uri in [
        "sftp://example.invalid/notes.txt",
        "smb://example.invalid/share/notes.txt",
    ] {
        assert_requires_uri_handlers(&gio::File::for_uri(uri));
    }

    assert!(requires_uri_handlers(&[native, trash]));
}

#[test]
fn fuse_backed_non_native_files_allow_path_handlers() {
    let Some(fuse) = mounted_non_native_file_with_path() else {
        eprintln!(
            "Skipping ui::open_with::tests::fuse_backed_non_native_files_allow_path_handlers: no GVfs FUSE mount with a local path"
        );
        return;
    };
    assert!(!fuse.is_native());
    assert!(fuse.path().is_some());
    assert!(!requires_uri_handlers(std::slice::from_ref(&fuse)));

    let trash = gio::File::for_uri("trash:///notes.txt");
    assert!(requires_uri_handlers(&[fuse.clone(), trash]));

    let app = path_only_app();
    launch_and_settle_recent(&app, std::slice::from_ref(&fuse), 1, true)
        .0
        .expect("FUSE path launch");
}

#[test]
fn path_only_launch_rejects_files_without_a_local_path() {
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
fn path_only_launch_allows_files_with_a_local_path() {
    let local = gio::File::for_path("/tmp/local.txt");
    let app = path_only_app();
    launch_and_settle_recent(&app, std::slice::from_ref(&local), 1, true)
        .0
        .expect("path launch");
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
    launch_and_settle_recent(&app, &files, files.len(), true)
        .0
        .expect("URI launch");
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
                OpenWithContext::Explicit,
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
            fn has_label(widget: &gtk::Widget, text: &str) -> bool {
                if widget
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text() == text)
                {
                    return true;
                }
                let mut child = widget.first_child();
                while let Some(widget) = child {
                    if has_label(&widget, text) {
                        return true;
                    }
                    child = widget.next_sibling();
                }
                false
            }
            assert!(
                !find_open(layer.upcast_ref())
                    .expect("Open button")
                    .is_sensitive()
            );
            assert!(has_label(
                layer.upcast_ref(),
                "No compatible applications were found."
            ));
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
                OpenWithContext::Explicit,
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

            search.set_text("VIEWER");
            assert_eq!(count_visible_rows(&list), 2);
            wait_until(&|| count_visible_rows(&list) == 2);
            assert!(open_btn.is_sensitive());

            search.set_text("nonexistent-app-xyz");
            assert!(!open_btn.is_sensitive());
            assert!(list.selected_row().is_none());
            wait_until(&|| !open_btn.is_sensitive());

            search.set_text("");
            wait_until(&|| open_btn.is_sensitive() && count_visible_rows(&list) == 5);

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
