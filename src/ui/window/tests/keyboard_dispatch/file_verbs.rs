// SPDX-License-Identifier: MIT

use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use super::*;
use crate::{
    model::{SortDirection, SortKey},
    ui::tenxer_mode::{Chord, Prompt},
};

mod actions;
mod open_with;
mod recent_apps;
mod rename;
mod sorting;

fn enable_tenxer(fixture: &KeyboardFixture) {
    let preferences = PreferenceManager::shared();
    fixture.shortcuts.bind_preferences(&preferences);
    preferences.set_filter_include_subfolders(false);
    preferences.set_tenxer_mode(true);
    pump(50);
    fixture.view.browser().clear_active_selection();
}

fn plain(fixture: &KeyboardFixture, key: Key) {
    assert!(fixture.press(key, ModifierType::empty()), "{key:?}");
}

fn shifted(fixture: &KeyboardFixture, key: Key) {
    assert!(
        fixture.press(key, ModifierType::SHIFT_MASK),
        "Shift+{key:?}"
    );
}

fn feedback(fixture: &KeyboardFixture) -> String {
    let text = fixture.shortcuts.feedback_text();
    fixture.shortcuts.dismiss_feedback();
    text
}

fn contents(path: &Path) -> String {
    std::fs::read_to_string(path).expect("fixture contents")
}

fn names_at(browser: &crate::app::Browser, depth: usize) -> Vec<String> {
    let count = browser
        .column_snapshot(depth)
        .map_or(0, |column| column.count);
    browser
        .with_entries(depth, 0..count, |entries| {
            entries
                .iter()
                .map(|entry| entry.display_name.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn lists(browser: &crate::app::Browser, name: &str) -> bool {
    let loaded = browser
        .column_snapshot(0)
        .is_some_and(|column| !column.loading);
    loaded && names_at(browser, 0).iter().any(|entry| entry == name)
}

fn refresh(fixture: &KeyboardFixture, name: &str) {
    plain(fixture, Key::F5);
    let browser = fixture.view.browser();
    // The reload streams its rows; commands must not race the rest of them.
    wait_until(|| lists(&browser, name));
    pump(100);
    wait_loaded(&browser, 0);
    focus_files(fixture);
}

fn widget_children(widget: &gtk::Widget) -> impl Iterator<Item = gtk::Widget> {
    std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
}

fn label_texts(widget: &gtk::Widget) -> Vec<String> {
    let mut texts = Vec::new();
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        texts.push(label.text().to_string());
    }
    for child in widget_children(widget) {
        texts.extend(label_texts(&child));
    }
    texts
}

fn close_dialog(fixture: &KeyboardFixture) {
    assert!(click_class(&fixture.overlay, "action-dialog-close"));
    wait_until(|| !modal_visible(&fixture.overlay));
    pump(200);
}

type Sections = Vec<(String, Vec<String>)>;

/// The Open With list as (heading, application names) for the rows `include` keeps.
fn sections_of(overlay: &gtk::Overlay, include: impl Fn(&gtk::Widget) -> bool) -> Sections {
    let Some(list) = widget_with_class(overlay.upcast_ref(), "open-with-list") else {
        return Vec::new();
    };
    let mut sections: Sections = Vec::new();
    for row in widget_children(&list).filter(|row| include(row)) {
        let labels = label_texts(&row);
        if row.has_css_class("open-with-heading-row") {
            sections.push((labels.join(""), Vec::new()));
        } else if let (Some(section), Some(name)) = (sections.last_mut(), labels.first()) {
            section.1.push(name.clone());
        }
    }
    sections
}

fn chooser_sections(overlay: &gtk::Overlay) -> Sections {
    sections_of(overlay, |_| true)
}

fn visible_chooser_sections(overlay: &gtk::Overlay) -> Sections {
    sections_of(overlay, gtk::Widget::is_visible)
}

fn received(output: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(output)
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .collect()
}

fn has_app_id(app: &gtk::gio::AppInfo, wanted: &str) -> bool {
    app.id().is_some_and(|app_id| app_id.as_str() == wanted)
}

/// Waits until GIO lists the application `wanted` for `mime`.
fn installed_app(mime: &str, wanted: &str) -> gtk::gio::AppInfo {
    wait_until(|| {
        gtk::gio::AppInfo::all_for_type(mime)
            .iter()
            .any(|app| has_app_id(app, wanted))
    });
    gtk::gio::AppInfo::all_for_type(mime)
        .into_iter()
        .find(|app| has_app_id(app, wanted))
        .expect("fake application")
}

fn launch_recorder(id: &str, targets: &[(&Path, &str)]) {
    let wanted = format!("{id}.desktop");
    let files: Vec<gtk::gio::File> = targets
        .iter()
        .map(|(path, _)| gtk::gio::File::for_path(path))
        .collect();
    let types: Vec<String> = targets.iter().map(|(_, mime)| mime.to_string()).collect();
    for mime in &types {
        installed_app(mime, &wanted);
    }
    let app = installed_app(&types[0], &wanted);
    crate::ui::open_with::launch(&app, &files, &types, None::<&gtk::gio::AppLaunchContext>)
        .expect("fake launch records history");
}

fn open_single_with_dialog(fixture: &KeyboardFixture, browser: &crate::app::Browser, name: &str) {
    fixture.view.browser().clear_active_selection();
    move_to_named(fixture, browser, name);
    plain(fixture, Key::space);
    shifted(fixture, Key::O);
    wait_until(|| modal_visible(&fixture.overlay));
}

/// Makes the application `id` the default handler for text/plain.
fn make_default_text_editor(id: &str) {
    let associations = gtk::glib::user_config_dir().join("mimeapps.list");
    let mut mimeapps = std::fs::read_to_string(&associations).expect("mimeapps");
    mimeapps.push_str(&format!(
        "[Default Applications]\ntext/plain={id}.desktop;\n"
    ));
    std::fs::write(&associations, mimeapps).expect("default application");
}
