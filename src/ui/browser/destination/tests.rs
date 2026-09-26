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
            bar.begin_edit();
            field.set_text("Photos");
            assert!(bar.is_editing());
            bar.select_directory(&teaching);
            assert!(!bar.is_editing());
            assert_eq!(field.text(), folder_input_path(&teaching));
        },
    );
}
