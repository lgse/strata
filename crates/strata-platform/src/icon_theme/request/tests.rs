use std::path::PathBuf;

use super::{IconRequest, IconSource, themed_names};

#[test]
fn gio_adds_symbolic_counterparts_after_the_names() {
    assert_eq!(
        themed_names(&["firefox"], false),
        ["firefox", "firefox-symbolic"]
    );
    assert_eq!(
        themed_names(&["drive-removable-media-symbolic", "drive"], false),
        [
            "drive-removable-media-symbolic",
            "drive",
            "drive-removable-media",
            "drive-symbolic"
        ]
    );
    // A repeat, plain or symbolic, is left out.
    assert_eq!(
        themed_names(&["a", "a-symbolic", "a"], false),
        ["a", "a-symbolic"]
    );
}

#[test]
fn dash_fallbacks_shorten_each_name() {
    assert_eq!(
        themed_names(&["text-x-python"], true),
        [
            "text-x-python",
            "text-x",
            "text",
            "text-x-python-symbolic",
            "text-x-symbolic",
            "text-symbolic"
        ]
    );
    // A shorter form already there stops the shortening.
    assert_eq!(
        themed_names(&["a-b", "a-c-d"], true),
        [
            "a-b",
            "a",
            "a-c-d",
            "a-c",
            "a-b-symbolic",
            "a-symbolic",
            "a-c-d-symbolic",
            "a-c-symbolic"
        ]
    );
    assert_eq!(
        themed_names(&["go-up-symbolic"], true),
        ["go-up-symbolic", "go-symbolic", "go-up", "go"]
    );
}

#[test]
fn a_desktop_icon_value_reads_as_gdesktopappinfo_reads_it() {
    let names = |value| match IconRequest::from_desktop_icon(value, 22, 1.0).map(|r| r.source) {
        Some(IconSource::Names(names)) => names,
        other => panic!("{value}: {other:?}"),
    };
    assert_eq!(
        names("org.gnome.Nautilus"),
        ["org.gnome.Nautilus", "org.gnome.Nautilus-symbolic"]
    );
    assert_eq!(names(" firefox.png "), ["firefox", "firefox-symbolic"]);
    assert_eq!(names("app.xpm"), ["app", "app-symbolic"]);
    assert_eq!(names("app.svgz"), ["app.svgz", "app.svgz-symbolic"]);
    assert_eq!(
        IconRequest::from_desktop_icon("/opt/app/icon.png", 22, 2.0).map(|r| r.source),
        Some(IconSource::File(PathBuf::from("/opt/app/icon.png")))
    );
    assert_eq!(IconRequest::from_desktop_icon("  ", 22, 1.0), None);
}

#[test]
fn a_fractional_scale_matches_the_next_whole_scale() {
    let at = |size, scale| {
        let r = IconRequest::named(["a"], size, scale);
        (r.pixels(), r.theme_scale())
    };
    assert_eq!(at(22, 1.0), (22, 1));
    assert_eq!(at(22, 1.25), (28, 2));
    assert_eq!(at(22, 2.0), (44, 2));
    assert_eq!(at(600, 2.0), (1024, 2));
    assert_eq!(at(22, f32::NAN), (1, 1));
}
