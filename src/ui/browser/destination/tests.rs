// SPDX-License-Identifier: MIT

use super::*;

fn crumb_labels(crumbs: &[DestinationCrumb]) -> Vec<&str> {
    crumbs.iter().map(|crumb| crumb.label.as_str()).collect()
}

fn crumb_kinds(crumbs: &[DestinationCrumb]) -> Vec<DestinationCrumbKind> {
    crumbs.iter().map(|crumb| crumb.kind).collect()
}

#[test]
fn nested_home_path_orders_ancestors_with_current_last() {
    let fixture = tempfile::tempdir().expect("breadcrumb fixture");
    let home = fixture.path().join("home");

    let crumbs = destination_crumbs("~/Projects/strata/", &home, &home, None, None, None, &home);

    assert_eq!(crumb_labels(&crumbs), ["~", "Projects", "strata"]);
    assert_eq!(
        crumb_kinds(&crumbs),
        [
            DestinationCrumbKind::Ancestor,
            DestinationCrumbKind::Ancestor,
            DestinationCrumbKind::Current,
        ]
    );
    assert_eq!(crumbs[0].target, home);
    assert_eq!(crumbs[2].target, home.join("Projects/strata"));
}

#[test]
fn absolute_path_outside_home_starts_at_fs_root() {
    let home = Path::new("/home/example");

    let crumbs = destination_crumbs("/tmp/share/", home, home, None, None, None, home);

    assert_eq!(crumb_labels(&crumbs), ["/", "tmp", "share"]);
    assert_eq!(
        crumbs.last().map(|crumb| crumb.kind),
        Some(DestinationCrumbKind::Current)
    );
}

#[test]
fn confined_nested_path_starts_at_device_root() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = tempfile::tempdir().expect("device fixture");
    let root = fixture.path().join("VANIA");
    std::fs::create_dir_all(root.join("Teaching/2026"))?;
    let canonical_root = std::fs::canonicalize(&root)?;

    let input = format!("{}/", root.join("Teaching/2026").display());
    let crumbs = destination_crumbs(
        &input,
        &root,
        &root,
        Some(&root),
        Some(&canonical_root),
        Some("VANIA"),
        Path::new("/home/example"),
    );

    assert_eq!(crumb_labels(&crumbs), ["VANIA", "Teaching", "2026"]);
    assert_eq!(
        crumbs.last().map(|crumb| crumb.kind),
        Some(DestinationCrumbKind::Current)
    );
    assert_eq!(crumbs[0].target, canonical_root);
    for crumb in &crumbs {
        assert!(
            crumb.target.strip_prefix(&canonical_root).is_ok(),
            "no breadcrumb target may exist above the device root"
        );
    }
    Ok(())
}

#[test]
fn confined_root_yields_single_current_crumb() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = tempfile::tempdir().expect("device fixture");
    let root = fixture.path().join("VANIA");
    std::fs::create_dir_all(&root)?;
    let canonical_root = std::fs::canonicalize(&root)?;

    let input = format!("{}/", root.display());
    let crumbs = destination_crumbs(
        &input,
        &root,
        &root,
        Some(&root),
        Some(&canonical_root),
        Some("VANIA"),
        Path::new("/home/example"),
    );

    assert_eq!(crumbs.len(), 1);
    assert_eq!(crumbs[0].label, "VANIA");
    assert_eq!(crumbs[0].kind, DestinationCrumbKind::Current);
    assert_eq!(crumbs[0].target, canonical_root);
    Ok(())
}

#[test]
fn escape_attempts_fall_back_to_device_root() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = tempfile::tempdir().expect("device fixture");
    let root = fixture.path().join("VANIA");
    std::fs::create_dir_all(&root)?;
    std::fs::create_dir_all(fixture.path().join("outside"))?;
    let canonical_root = std::fs::canonicalize(&root)?;

    for input in [
        format!("{}/", root.join("../outside").display()),
        format!("{}/", root.join("NoSuch").display()),
    ] {
        let crumbs = destination_crumbs(
            &input,
            &root,
            &root,
            Some(&root),
            Some(&canonical_root),
            Some("VANIA"),
            Path::new("/home/example"),
        );
        assert_eq!(crumb_labels(&crumbs), ["VANIA"], "input {input:?}");
        assert_eq!(crumbs[0].kind, DestinationCrumbKind::Scope);
        assert_eq!(crumbs[0].target, canonical_root);
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn escaping_symlink_falls_back_to_device_root() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = tempfile::tempdir().expect("device fixture");
    let root = fixture.path().join("VANIA");
    std::fs::create_dir_all(&root)?;
    let outside = fixture.path().join("outside");
    std::fs::create_dir_all(&outside)?;
    std::os::unix::fs::symlink(&outside, root.join("link"))?;
    let canonical_root = std::fs::canonicalize(&root)?;

    let input = format!("{}/", root.join("link").display());
    let crumbs = destination_crumbs(
        &input,
        &root,
        &root,
        Some(&root),
        Some(&canonical_root),
        Some("VANIA"),
        Path::new("/home/example"),
    );

    assert_eq!(crumb_labels(&crumbs), ["VANIA"]);
    assert_eq!(crumbs[0].kind, DestinationCrumbKind::Scope);
    assert_eq!(crumbs[0].target, canonical_root);
    Ok(())
}

#[test]
fn fuzzy_send_to_shows_scope_root() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = tempfile::tempdir().expect("device fixture");
    let root = fixture.path().join("VANIA");
    std::fs::create_dir_all(&root)?;

    let crumbs = destination_crumbs(
        "Photos",
        &root,
        &root,
        Some(&root),
        None,
        Some("VANIA"),
        Path::new("/home/example"),
    );

    assert_eq!(crumbs.len(), 1);
    assert_eq!(crumbs[0].label, "VANIA");
    assert_eq!(crumbs[0].kind, DestinationCrumbKind::Scope);
    assert_eq!(crumbs[0].target, root);
    Ok(())
}

#[test]
fn fuzzy_unconfined_search_shows_home_scope() {
    let home = Path::new("/home/example");

    let crumbs = destination_crumbs("Photos", home, home, None, None, None, home);

    assert_eq!(crumbs.len(), 1);
    assert_eq!(crumbs[0].label, "~");
    assert_eq!(crumbs[0].kind, DestinationCrumbKind::Scope);
    assert_eq!(crumbs[0].target, Path::new("/home/example"));
}

#[test]
fn location_bar_mirrors_entry_error_state() {
    crate::test_support::gtk_test(
        "ui::browser::destination::tests::location_bar_mirrors_entry_error_state",
        || {
            use gtk::prelude::*;
            let fixture = tempfile::tempdir().expect("location bar fixture");
            let base = fixture.path().join("base");
            let field = gtk::Entry::new();
            field.set_text(&format!("{}/", base.display()));
            let bar = DestinationLocationBar::wrap(
                field.clone(),
                base.clone(),
                fixture.path().to_path_buf(),
                None,
                None,
            );
            let stack = bar
                .widget()
                .downcast::<gtk::Stack>()
                .expect("location stack");
            assert!(!stack.has_css_class("error"));
            field.add_css_class("error");
            assert!(
                stack.has_css_class("error"),
                "the outer bar owns the error chrome"
            );
            field.remove_css_class("error");
            assert!(!stack.has_css_class("error"));
        },
    );
}

#[test]
fn location_bar_escape_restores_edit_start_text() {
    crate::test_support::gtk_test(
        "ui::browser::destination::tests::location_bar_escape_restores_edit_start_text",
        || {
            use gtk::prelude::*;
            let fixture = tempfile::tempdir().expect("location bar fixture");
            let base = fixture.path().join("base");
            let start_text = format!("{}/", base.display());
            let field = gtk::Entry::new();
            field.set_text(&start_text);
            let bar = DestinationLocationBar::wrap(
                field.clone(),
                base.clone(),
                fixture.path().to_path_buf(),
                None,
                None,
            );
            assert!(!bar.is_editing());
            bar.begin_edit();
            assert!(bar.is_editing());
            field.set_text("Nope");
            assert!(matches!(
                bar.handle_key(gtk::gdk::Key::Escape, gtk::gdk::ModifierType::empty()),
                glib::Propagation::Stop
            ));
            assert_eq!(field.text(), start_text);
            assert!(!bar.is_editing());
            assert!(matches!(
                bar.handle_key(gtk::gdk::Key::Escape, gtk::gdk::ModifierType::empty()),
                glib::Propagation::Proceed
            ));
            assert!(matches!(
                bar.handle_key(gtk::gdk::Key::Return, gtk::gdk::ModifierType::empty()),
                glib::Propagation::Proceed
            ));
        },
    );
}

#[test]
fn location_bar_selection_returns_to_browse() {
    crate::test_support::gtk_test(
        "ui::browser::destination::tests::location_bar_selection_returns_to_browse",
        || {
            use gtk::prelude::*;
            let fixture = tempfile::tempdir().expect("location bar fixture");
            let base = fixture.path().join("base");
            let teaching = base.join("Teaching");
            let field = gtk::Entry::new();
            field.set_text(&format!("{}/", base.display()));
            let bar = DestinationLocationBar::wrap(
                field.clone(),
                base.clone(),
                fixture.path().to_path_buf(),
                None,
                None,
            );
            std::fs::create_dir_all(&teaching).expect("destination folders");
            bar.begin_edit();
            field.set_text("Photos");
            assert!(bar.is_editing());
            bar.select_directory(&teaching);
            assert!(!bar.is_editing());
            assert_eq!(field.text(), folder_input_path(&teaching));

            bar.go_back();
            assert_eq!(
                field.text(),
                folder_input_path(&base),
                "Back skips typed text and returns to the previous folder"
            );
            bar.go_forward();
            assert_eq!(field.text(), folder_input_path(&teaching));
            bar.go_up();
            assert_eq!(field.text(), folder_input_path(&base));
            bar.go_forward();
            assert_eq!(
                field.text(),
                folder_input_path(&base),
                "Up is a new visit and drops the forward trail"
            );
            bar.go_back();
            assert_eq!(field.text(), folder_input_path(&teaching));
        },
    );
}

#[test]
fn history_back_forward_round_trip() {
    let (a, b, c) = (Path::new("/a"), Path::new("/b"), Path::new("/c"));
    let mut history = DestinationHistory::default();
    history.visit(a, a);
    assert_eq!(
        history.go_back(a),
        None,
        "revisiting the current folder records nothing"
    );
    history.visit(a, b);
    history.visit(b, c);
    assert_eq!(history.go_back(c).as_deref(), Some(b));
    assert_eq!(history.go_back(b).as_deref(), Some(a));
    assert_eq!(history.go_back(a), None);
    assert_eq!(history.go_forward(a).as_deref(), Some(b));
    history.visit(b, a);
    assert_eq!(
        history.go_forward(a),
        None,
        "a new visit drops the forward trail"
    );
    assert_eq!(history.go_back(a).as_deref(), Some(b));
}

#[cfg(unix)]
#[test]
fn up_stops_at_search_filesystem_and_device_roots() -> Result<(), Box<dyn std::error::Error>> {
    let home = Path::new("/home/example");
    let unconfined = |input: &str| parent_destination(input, home, None, None, home);
    assert_eq!(
        unconfined("~/Projects/strata/"),
        Some(home.join("Projects"))
    );
    assert_eq!(unconfined("~/"), Some(PathBuf::from("/home")));
    assert_eq!(unconfined("/"), None);
    assert_eq!(unconfined("Photos"), None, "name search has no parent");

    let fixture = tempfile::tempdir().expect("device fixture");
    let root = fixture.path().join("VANIA");
    std::fs::create_dir_all(root.join("Teaching/2026"))?;
    let outside = fixture.path().join("outside");
    std::fs::create_dir_all(&outside)?;
    std::os::unix::fs::symlink(&outside, root.join("link"))?;
    let canonical_root = std::fs::canonicalize(&root)?;
    let confined = |path: PathBuf| {
        parent_destination(
            &format!("{}/", path.display()),
            &root,
            Some(&root),
            Some(&canonical_root),
            home,
        )
    };
    assert_eq!(
        confined(root.join("Teaching/2026")),
        Some(canonical_root.join("Teaching"))
    );
    assert_eq!(confined(root.clone()), None, "Up stops at the device root");
    assert_eq!(confined(root.join("link")), None);
    assert_eq!(confined(root.join("NoSuch")), None);
    Ok(())
}

#[test]
fn path_suggestions_tell_an_empty_folder_from_a_failed_match()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = tempfile::tempdir().expect("suggestion fixture");
    let base = fixture.path().join("base");
    std::fs::create_dir_all(base.join("Alpha"))?;
    std::fs::create_dir_all(base.join("leaf"))?;
    std::fs::write(base.join("leaf/notes.txt"), b"notes")?;
    let home = Path::new("/home/example");
    let suggest = |relative: &str| {
        path_suggestions(&format!("{}/{relative}", base.display()), &base, home, None)
    };

    let listed = suggest("");
    assert_eq!(listed.paths, [base.join("Alpha"), base.join("leaf")]);

    let leaf = suggest("leaf/");
    assert!(leaf.paths.is_empty());
    assert_eq!(leaf.empty, EmptySuggestions::NoSubfolders);

    for failed in ["zz", "missing/"] {
        let result = suggest(failed);
        assert!(result.paths.is_empty(), "input {failed:?}");
        assert_eq!(
            result.empty,
            EmptySuggestions::NoMatches,
            "input {failed:?}"
        );
    }
    Ok(())
}

#[test]
fn down_from_the_entry_focuses_the_first_folder() {
    crate::test_support::gtk_test(
        "ui::browser::destination::tests::down_from_the_entry_focuses_the_first_folder",
        || {
            use gtk::prelude::*;
            let fixture = tempfile::tempdir().expect("picker fixture");
            let base = fixture.path().join("base");
            std::fs::create_dir_all(base.join("Alpha")).expect("first folder");
            std::fs::create_dir_all(base.join("Beta")).expect("second folder");
            let picker = DestinationBrowser::new(
                DestinationBrowserOptions {
                    base: base.clone(),
                    search_root: fixture.path().to_path_buf(),
                    root_limit: None,
                    root_label: None,
                    show_hidden: false,
                    places: Vec::new(),
                },
                |_| {},
            );
            let window = gtk::Window::builder().child(&picker.widget()).build();
            window.present();
            picker.activate();
            assert!(matches!(
                picker
                    .bar
                    .handle_key(gtk::gdk::Key::Down, gtk::gdk::ModifierType::empty()),
                glib::Propagation::Proceed
            ));
            picker.bar.begin_edit();
            let first = base.join("Alpha");
            let listed = || find_named(&picker.root, &first.to_string_lossy()).is_some();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !listed() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out listing the starting folder"
                );
                glib::MainContext::default().iteration(false);
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(matches!(
                picker
                    .bar
                    .handle_key(gtk::gdk::Key::Down, gtk::gdk::ModifierType::empty()),
                glib::Propagation::Stop
            ));
            assert_eq!(
                gtk::prelude::GtkWindowExt::focus(&window)
                    .map(|focus| focus.widget_name().to_string()),
                Some(first.to_string_lossy().into_owned()),
                "Down moves from the entry to the first listed folder"
            );
            window.destroy();
        },
    );
}

fn find_named(root: &impl IsA<gtk::Widget>, name: &str) -> Option<gtk::Widget> {
    let mut pending = vec![root.clone().upcast::<gtk::Widget>()];
    while let Some(widget) = pending.pop() {
        if widget.widget_name() == name {
            return Some(widget);
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            pending.push(next);
        }
    }
    None
}
