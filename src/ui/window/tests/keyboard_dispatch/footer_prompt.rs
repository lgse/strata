// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::tenxer_mode::Prompt;

mod filter;
mod filter_slash;
mod find;
mod search;
mod search_hits;

const ALL_MODES: [BrowserMode; 3] = [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons];

fn highlighted_labels(widget: &gtk::Widget) -> Vec<(String, gtk::pango::AttrList)> {
    fn collect(widget: &gtk::Widget, labels: &mut Vec<(String, gtk::pango::AttrList)>) {
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && let Some(attributes) = label.attributes()
        {
            labels.push((label.text().to_string(), attributes));
        }
        if let Some(label) = widget.downcast_ref::<gtk::Inscription>()
            && let Some(attributes) = label.attributes()
        {
            labels.push((label.text().unwrap_or_default().to_string(), attributes));
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            collect(&current, labels);
            child = current.next_sibling();
        }
    }
    let mut labels = Vec::new();
    collect(widget, &mut labels);
    labels
}

pub(super) fn highlighted_names(widget: &gtk::Widget) -> Vec<String> {
    let mut names: Vec<_> = highlighted_labels(widget)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    names.sort();
    names.dedup();
    names
}

fn highlight_colors(widget: &gtk::Widget) -> Vec<String> {
    highlighted_labels(widget)
        .into_iter()
        .flat_map(|(_, attributes)| attributes.attributes())
        .filter_map(|attribute| {
            let color = attribute.downcast_ref::<gtk::pango::AttrColor>()?;
            Some(color.color().to_str().to_string())
        })
        .collect()
}

fn wait_highlights(fixture: &KeyboardFixture, expected: &[&str]) {
    wait_until(|| highlighted_names(&fixture.view.widget()) == expected);
}

pub(super) fn type_and_submit(fixture: &KeyboardFixture, prompt: Key, text: &str) {
    assert!(fixture.press(prompt, ModifierType::empty()));
    assert!(fixture.shortcuts.prompt_has_focus());
    fixture.shortcuts.prompt().set_text(text);
    assert!(fixture.press(Key::Return, ModifierType::empty()));
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
}

pub(super) fn enable_tenxer(fixture: &KeyboardFixture) -> Rc<PreferenceManager> {
    let preferences = PreferenceManager::shared();
    fixture.shortcuts.bind_preferences(&preferences);
    preferences.set_tenxer_mode(true);
    pump(50);
    preferences
}

/// A seeded fixture in 10xer mode whose filter stays in the open folder.
fn tenxer_filter_fixture() -> (KeyboardFixture, Rc<PreferenceManager>) {
    let fixture = KeyboardFixture::new();
    seed_filter_tree(&fixture);
    let preferences = enable_tenxer(&fixture);
    preferences.set_filter_include_subfolders(false);
    (fixture, preferences)
}

/// Adds alpha-report.txt, beta.txt, gamma-report.md, and a reports folder
/// holding deep-report.txt. "report" names two files and the folder here, and
/// the nested file only with Include subfolders on.
pub(super) fn seed_filter_tree(fixture: &KeyboardFixture) {
    let root = fixture._directory.path();
    for name in ["alpha-report.txt", "beta.txt", "gamma-report.md"] {
        std::fs::write(root.join(name), b"filter").expect("fixture file");
    }
    std::fs::create_dir(root.join("reports")).expect("fixture folder");
    std::fs::write(root.join("reports/deep-report.txt"), b"filter").expect("nested file");
    let browser = fixture.view.browser();
    fixture.view.refresh();
    wait_loaded(&browser, 0);
    wait_until(|| entry_count(&browser) == 7);
    fixture.shortcuts.observe_browser(&browser);
}

pub(super) const IMMEDIATE_REPORTS: [&str; 3] = ["alpha-report.txt", "gamma-report.md", "reports"];
pub(super) const ALL_REPORTS: [&str; 4] = [
    "alpha-report.txt",
    "deep-report.txt",
    "gamma-report.md",
    "reports",
];

fn result_names(fixture: &KeyboardFixture) -> Vec<String> {
    let mut names = fixture.view.filter_result_names();
    names.sort();
    names
}

pub(super) fn wait_results(fixture: &KeyboardFixture, expected: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while result_names(fixture) != expected {
        assert!(
            Instant::now() < deadline,
            "filter results {:?} never became {expected:?}",
            result_names(fixture)
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub(super) fn selected_result_names(fixture: &KeyboardFixture) -> Vec<String> {
    let mut names: Vec<_> = fixture
        .view
        .selected_search_results()
        .unwrap_or_default()
        .into_iter()
        .map(|entry| entry.display_name)
        .collect();
    names.sort();
    names
}

fn revealed_filter_funnels(widget: &gtk::Widget) -> usize {
    let own = widget
        .downcast_ref::<gtk::Revealer>()
        .is_some_and(|revealer| {
            revealer.has_css_class("tenxer-filter-revealer") && revealer.reveals_child()
        });
    let mut count = usize::from(own);
    let mut child = widget.first_child();
    while let Some(current) = child {
        count += revealed_filter_funnels(&current);
        child = current.next_sibling();
    }
    count
}

pub(super) fn commit_filter(fixture: &KeyboardFixture, text: &str) {
    type_and_submit(fixture, Key::f, text);
    wait_until(|| fixture.view.item_view_has_focus());
}

fn wait_footer(fixture: &KeyboardFixture, what: &str, condition: impl Fn() -> bool) {
    wait_until_or(condition, || {
        format!(
            "{what}: footer count {:?}, mark {:?}, hit {:?}",
            fixture.shortcuts.count_text(),
            fixture.shortcuts.filter_mark(),
            fixture.shortcuts.current_hit(),
        )
    });
}

fn fixture_name(fixture: &KeyboardFixture) -> &str {
    fixture
        ._directory
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("fixture name")
}

fn in_fixture_folder(fixture: &KeyboardFixture) -> bool {
    location_ends_with(
        fixture.view.browser().active_location(),
        fixture_name(fixture),
    )
}

fn type_search(fixture: &KeyboardFixture, text: &str) {
    assert!(fixture.press(Key::s, ModifierType::empty()));
    assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Search));
    fixture.shortcuts.prompt().set_text(text);
}

pub(super) fn commit_search(fixture: &KeyboardFixture, text: &str) {
    type_search(fixture, text);
    assert!(fixture.press(Key::Return, ModifierType::empty()));
    assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
    wait_until(|| fixture.view.item_view_has_focus());
}

fn wait_filter_restored(fixture: &KeyboardFixture, query: &str, results: &[&str]) {
    wait_results(fixture, results);
    wait_until(|| !fixture.view.listing_search_active());
    assert_eq!(fixture.view.listing_filter().as_deref(), Some(query));
    wait_until(|| fixture.shortcuts.filter_mark() == Some(format!("filter: {query}")));
    assert_eq!(
        fixture.shortcuts.current_hit(),
        None,
        "filters show no hit path"
    );
}

/// Opens the `reports` hit with Enter, then goes back to the fixture folder.
fn enter_reports_and_back(fixture: &KeyboardFixture) {
    let browser = fixture.view.browser();
    assert!(fixture.press(Key::Return, ModifierType::empty()));
    wait_until(|| location_ends_with(browser.active_location(), "reports"));
    browser.back();
    wait_until(|| in_fixture_folder(fixture));
    wait_loaded(&browser, 0);
}

pub(super) fn hit_cursor(fixture: &KeyboardFixture) -> Option<String> {
    fixture
        .view
        .selected_search_result()
        .map(|entry| entry.display_name)
}

fn hidden_selection(fixture: &KeyboardFixture) -> Vec<String> {
    fixture
        .view
        .browser()
        .selected_entries()
        .into_iter()
        .map(|entry| entry.display_name)
        .collect()
}

/// Moves the cursor onto `name` with real keys where the view has a linear
/// order; Icons focus the hit directly.
pub(super) fn cursor_to_hit(fixture: &KeyboardFixture, name: &str) {
    if fixture.view.view_mode() == BrowserMode::Icons {
        let path = fixture._directory.path().join(hit_path(name));
        assert!(fixture.view.focus_search_result(&path), "{name}");
    } else {
        assert!(fixture.press(Key::g, ModifierType::empty()));
        assert!(fixture.press(Key::g, ModifierType::empty()));
        for _ in ALL_REPORTS {
            if hit_cursor(fixture).as_deref() == Some(name) {
                break;
            }
            assert!(fixture.press(Key::j, ModifierType::empty()));
        }
    }
    wait_until(|| hit_cursor(fixture).as_deref() == Some(name));
}

/// The path, relative to the fixture root, of a seeded hit.
fn hit_path(name: &str) -> String {
    if name == "deep-report.txt" {
        "reports/deep-report.txt".into()
    } else {
        name.into()
    }
}

fn wait_footer_hit(fixture: &KeyboardFixture, mode: BrowserMode, after: &str, name: &str) {
    let path = hit_path(name);
    wait_footer(
        fixture,
        &format!("{mode:?} after {after}: cursor on {path:?}"),
        || fixture.shortcuts.current_hit() == Some((path.clone(), path.clone())),
    );
}
