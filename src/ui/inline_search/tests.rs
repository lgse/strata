// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::{
    fs,
    time::{Instant, SystemTime},
};

fn labels(widget: &gtk::Widget) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        result.push(label.text().to_string());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        result.extend(labels(&widget));
    }
    result
}

fn wait_until(condition: impl Fn() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "search did not update"
        );
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn alternate_view_search_finds_descendants_and_restores_the_original_view() {
    const CHILD: &str = "STRATA_ALTERNATE_SEARCH_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "ui::inline_search::tests::alternate_view_search_finds_descendants_and_restores_the_original_view"])
            .env(CHILD, "1").status().expect("start GTK test");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }
    let id = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let fixture = std::env::temp_dir().join(format!("strata-alternate-search-{id}"));
    let root = fixture.join("Documents");
    fs::create_dir_all(root.join("github/strata")).expect("create nested directory");
    fs::create_dir_all(fixture.join("outside-strata")).expect("create sibling directory");
    fs::write(root.join("github/readme.txt"), "fixture").expect("create second match");
    let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
    let entry = gtk::Entry::new();
    let original = gtk::Label::new(Some("Original view"));
    let widget = wrap(&original, &entry, Some(root.clone()), &browser);
    let stack = widget
        .clone()
        .downcast::<gtk::Stack>()
        .expect("local search stack");
    entry.set_text("stra");
    wait_until(|| labels(&widget).contains(&root.join("github/strata").display().to_string()));
    assert!(
        !labels(&widget)
            .iter()
            .any(|text| text.contains("outside-strata"))
    );
    entry.set_text("readme");
    wait_until(|| labels(&widget).contains(&root.join("github/readme.txt").display().to_string()));
    entry.set_text("");
    wait_until(|| stack.visible_child_name().as_deref() == Some("files"));
    assert_eq!(stack.visible_child(), Some(original.upcast()));
    entry.set_text("stra");
    wait_until(|| labels(&widget).contains(&root.join("github/strata").display().to_string()));
    let controllers = entry.observe_controllers();
    let keys = (0..controllers.n_items())
        .filter_map(|index| controllers.item(index))
        .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        .expect("recursive search key controller");
    assert_eq!(keys.propagation_phase(), gtk::PropagationPhase::Capture);
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::Return,
            &0u32,
            &gtk::gdk::ModifierType::empty()
        ],
    ));
    assert_eq!(
        browser.active_location(),
        Some(Location::local(root.join("github/strata")))
    );
    entry.set_text("");
    wait_until(|| stack.visible_child_name().as_deref() == Some("files"));
    fs::remove_dir_all(fixture).expect("remove fixture");
}
