// SPDX-License-Identifier: MIT

#![cfg(test)]

pub(crate) mod operations;
pub(crate) mod parked;
pub(crate) mod widgets;

use std::time::Duration;

use gtk::glib;
pub(crate) use strata_test_support::{
    ASYNC_MAIN_CONTEXT_DEFAULT, Gate, MainContextGuard, PrivateContext, TestMutex, capture_logs,
    captured_event, create_empty_file, distinct_device_dirs, serve_http_once, temp_base,
};

pub(crate) fn texture_has_visible_pixels(texture: &gtk::gdk::Texture) -> bool {
    use gtk::prelude::*;
    let stride = texture.width() as usize * 4;
    let mut pixels = vec![0; stride * texture.height() as usize];
    texture.download(&mut pixels, stride);
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| u32::from_ne_bytes(*pixel) >> 24 != 0)
}

pub(crate) fn save_pins(pins: &[(crate::model::Location, String)]) {
    let path = crate::adapters::bookmarks::pinned_places_path();
    std::fs::create_dir_all(path.parent().expect("bookmarks directory"))
        .expect("bookmarks directory");
    std::fs::write(
        path,
        crate::adapters::bookmarks::serialize_pinned_places(pins),
    )
    .expect("pins");
}

pub(crate) fn saved_pins() -> Vec<(crate::model::Location, String)> {
    let contents =
        std::fs::read(crate::adapters::bookmarks::pinned_places_path()).unwrap_or_default();
    crate::adapters::bookmarks::parse_pinned_places(&contents)
}

/// Text an input method commits into `entry`. The commit goes through the
/// field's own input method, which replaces a selection with it as typing does.
pub(crate) fn commit_text(entry: &gtk::Entry, text: &str) {
    use gtk::prelude::*;
    let field = entry.delegate().expect("an entry delegates to its text");
    let controllers = field.observe_controllers();
    let input_method = (0..controllers.n_items())
        .filter_map(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventControllerKey>()
        })
        .find_map(|keys| keys.im_context())
        .expect("a text field's input method");
    input_method.emit_by_name::<()>("commit", &[&text]);
}

pub(crate) fn present_window(window: &gtk::Window) {
    use gtk::prelude::*;

    window.present();
    let clock = window.frame_clock().expect("presented window frame clock");
    assert!(
        wait_for_paint(&clock),
        "presented window did not paint its first frame"
    );
}

/// Requests a frame from `clock` and runs the main context until it paints,
/// giving up after ten seconds. Returns whether the frame painted.
pub(crate) fn wait_for_paint(clock: &gtk::gdk::FrameClock) -> bool {
    use gtk::prelude::*;
    use std::{cell::Cell, rc::Rc};

    let painted = Rc::new(Cell::new(false));
    let ready = painted.clone();
    let handler = clock.connect_after_paint(move |_| ready.set(true));
    clock.request_phase(gtk::gdk::FrameClockPhase::PAINT);
    let expired = Rc::new(Cell::new(false));
    let timed_out = expired.clone();
    let timeout = glib::timeout_add_local_once(Duration::from_secs(10), move || {
        timed_out.set(true);
    });
    let context = glib::MainContext::default();
    while !painted.get() && !expired.get() {
        context.iteration(true);
    }
    clock.disconnect(handler);
    if !expired.get() {
        timeout.remove();
    }
    painted.get()
}

/// GTK initialization is thread-affine; each UI test gets a process and disposable preferences.
pub(crate) fn gtk_test(name: &str, run: impl FnOnce()) {
    gtk_test_with_env(name, std::iter::empty::<(&str, &std::ffi::OsStr)>(), run);
}

pub(crate) fn gtk_test_with_env(
    name: &str,
    extra_env: impl IntoIterator<Item = (impl AsRef<std::ffi::OsStr>, impl AsRef<std::ffi::OsStr>)>,
    run: impl FnOnce(),
) {
    const CHILD: &str = "STRATA_ISOLATED_GTK_TEST";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        if let Err(error) = gtk::init() {
            assert!(
                std::env::var_os("STRATA_REQUIRE_GTK_TESTS").is_none(),
                "GTK display required: {error}"
            );
            eprintln!("Skipping {name}: {error}");
            return;
        }
        crate::assets::prepare().expect("bundled assets");
        crate::assets::register_icon_theme();
        crate::adapters::mount_watch::start();
        crate::util::ui_thread::mark();
        run();
        return;
    }
    let extra_env: Vec<(std::ffi::OsString, std::ffi::OsString)> = extra_env
        .into_iter()
        .map(|(key, value)| (key.as_ref().to_owned(), value.as_ref().to_owned()))
        .collect();
    // Child processes still share the display's clipboard and pointer grabs.
    static DISPLAY: TestMutex = TestMutex::new();
    let _display = DISPLAY.lock().expect("GTK display lock");
    let sandbox = tempfile::tempdir().expect("isolated preferences");
    let home = sandbox.path().join("home");
    std::fs::create_dir_all(&home).expect("isolated home");
    let mut command = std::process::Command::new(std::env::current_exe().expect("test executable"));
    command
        // The parent already selected this exact case, including explicit --ignored runs.
        .args(["--exact", name, "--nocapture", "--include-ignored"])
        .env(CHILD, name)
        .env("HOME", home)
        .env("XDG_STATE_HOME", sandbox.path().join("state"))
        .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
        .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
        .env("XDG_DATA_HOME", sandbox.path().join("data"));
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let status = command.status().expect("isolated GTK test starts");
    assert!(status.success(), "{name} failed");
}
