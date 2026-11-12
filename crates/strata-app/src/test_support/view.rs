// SPDX-License-Identifier: MIT

//! What the single-pane views' tests share: a window opened in a saved mode, keys fed to it,
//! and the view read from the accessibility tree as the QA harness reads it.

use std::path::{Path, PathBuf};

use rune::{Key, KeyEvent, Mods, PhysicalInput};
use serde_json::Value;
use strata_components::{collection::element::row_key, contract::EntryId};
use strata_core::model::Location;

use super::{PATIENCE, hosted_after};
use crate::host::offscreen::Offscreen;

fn settings_file() -> PathBuf {
    strata_core::storage::config_directory().join("settings.toml")
}

/// Runs the test `name` of `module` in a window whose saved `browser_mode` is `mode`.
pub(crate) fn hosted_in(module: &str, mode: &'static str, name: &str, run: impl FnOnce(Offscreen)) {
    let prepare = move || {
        let file = settings_file();
        std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
        let saved =
            format!("mode = \"theme\"\ntheme = \"azure-glow\"\nbrowser_mode = \"{mode}\"\n");
        std::fs::write(file, saved).expect("settings");
    };
    hosted_after(module, name, prepare, run);
}

/// How a view looks to assistive technology: what its pane is described as, its list's role,
/// and its entries' role.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shape {
    pub(crate) described: &'static str,
    pub(crate) list: &'static str,
    pub(crate) entry: &'static str,
}

/// A view as assistive technology hears it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Seen {
    /// The group around the list: its name and description.
    pub(crate) pane: Option<[String; 2]>,
    /// The list: its name and description.
    pub(crate) list: Option<[String; 2]>,
    /// Each entry's name and description.
    pub(crate) entries: Vec<[String; 2]>,
    pub(crate) selected: Vec<String>,
    /// The list's active descendant: where the keyboard is.
    pub(crate) focused: Option<String>,
}

impl Seen {
    pub(crate) fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|[name, _]| name.as_str()).collect()
    }
}

pub(crate) fn text(node: &Value, field: &str) -> String {
    node[field].as_str().unwrap_or_default().to_owned()
}

fn heard(node: &Value) -> [String; 2] {
    [text(node, "name"), text(node, "description")]
}

fn find<'a>(node: &'a Value, wanted: &dyn Fn(&Value) -> bool, found: &mut Vec<&'a Value>) {
    for child in node["children"].as_array().map_or(&[][..], Vec::as_slice) {
        if wanted(child) {
            found.push(child);
        }
        find(child, wanted, found);
    }
}

pub(crate) fn all<'a>(node: &'a Value, wanted: &dyn Fn(&Value) -> bool) -> Vec<&'a Value> {
    let mut found = Vec::new();
    find(node, wanted, &mut found);
    found
}

fn has_state(node: &Value, state: &str) -> bool {
    let states = node["states"].as_array();
    states.is_some_and(|states| states.iter().any(|held| held == state))
}

/// Whether `node` is keyed as the collection keys an entry's row ([`row_key`]): a list holds
/// other nodes of an entry's role, such as a table's headings.
fn keyed_as_entry(node: &Value) -> bool {
    let key = node["key"].as_str().unwrap_or_default();
    let id = key
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .parse();
    id.is_ok_and(|id| row_key(EntryId::new(id)) == key)
}

pub(crate) fn seen(host: &Offscreen, shape: Shape) -> Seen {
    let tree = host.tree();
    let pane = all(&tree, &|node| node["description"] == shape.described);
    let Some(pane) = pane.first() else {
        return Seen::default();
    };
    let list = all(pane, &|node| node["role"] == shape.list);
    let Some(list) = list.first() else {
        return Seen::default();
    };
    let entries = all(list, &|node| {
        node["role"] == shape.entry && keyed_as_entry(node)
    });
    let active = &list["activeDescendant"];
    Seen {
        pane: Some(heard(pane)),
        list: Some(heard(list)),
        entries: entries.iter().map(|entry| heard(entry)).collect(),
        selected: entries
            .iter()
            .filter(|entry| has_state(entry, "selected"))
            .map(|entry| text(entry, "name"))
            .collect(),
        focused: entries
            .iter()
            .find(|entry| entry["key"] == *active)
            .map(|entry| text(entry, "name")),
    }
}

/// Turns the host until neither GLib nor Rune has anything ready, landing each motion.
pub(crate) fn idle(host: &mut Offscreen) {
    while host.turn() {
        host.runtime().settle_all();
    }
}

pub(crate) fn wait(
    host: &mut Offscreen,
    shape: Shape,
    what: &str,
    done: impl Fn(&Seen) -> bool,
) -> Seen {
    let reached = host.run_until(PATIENCE, |host| done(&seen(host, shape)));
    let seen = seen(host, shape);
    assert!(reached, "waited for {what}; the tree shows {seen:#?}");
    seen
}

/// Opens `folder` and waits for its entries.
pub(crate) fn open(host: &mut Offscreen, shape: Shape, folder: &Path) -> Seen {
    host.app().open(Location::local(folder));
    let listed = wait(host, shape, "the folder's listing", |seen| {
        !seen.entries.is_empty()
    });
    idle(host);
    listed
}

/// Feeds the host a key with its modifiers held, as a keyboard sends one.
pub(crate) fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    let runtime = host.runtime();
    runtime.physical_input(PhysicalInput::Modifiers(mods));
    runtime.key(KeyEvent { key, mods });
    runtime.physical_input(PhysicalInput::Modifiers(Mods::default()));
    idle(host);
}

pub(crate) fn shift() -> Mods {
    Mods {
        shift: true,
        ..Mods::default()
    }
}

/// What the browser holds: the names it has selected, and its focused entry's.
pub(crate) fn browser_holds(host: &Offscreen) -> (Vec<String>, Option<String>) {
    let browser = host.app().browser();
    let selected = browser.selected_entries();
    let selected = selected.into_iter().map(|entry| entry.display_name);
    let focused = browser.focused_entry().map(|entry| entry.display_name);
    (selected.collect(), focused)
}

/// The QA backend reads the selection and the cursor from the tree: the browser must hold
/// the same.
pub(crate) fn assert_browser_agrees(host: &Offscreen, shape: Shape, after: &str) {
    let seen = seen(host, shape);
    assert_eq!(
        browser_holds(host),
        (seen.selected, seen.focused),
        "after {after}"
    );
}

/// The rename field, as assistive technology hears it: its text, and whether it has the
/// keyboard.
pub(crate) fn field(host: &Offscreen) -> Option<(String, bool)> {
    let tree = host.tree();
    let fields = all(&tree, &|node| node["role"] == "text-input");
    let field = fields.first()?;
    Some((text(field, "value"), has_state(field, "focused")))
}

/// The middle of the entry named `name`, in window points.
fn middle_of(host: &Offscreen, shape: Shape, name: &str) -> (f32, f32) {
    let tree = host.tree();
    let entries = all(&tree, &|node| {
        node["role"] == shape.entry && node["name"] == name && keyed_as_entry(node)
    });
    let bounds = &entries.first().expect("the entry")["bounds"];
    let at = |index: usize| bounds[index].as_f64().expect("a bound");
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    let middle = ((at(0) + at(2) / 2.0) as f32, (at(1) + at(3) / 2.0) as f32);
    middle
}

/// A point of the view's list below its entries, in window points.
fn blank_of(host: &Offscreen, shape: Shape) -> (f32, f32) {
    let tree = host.tree();
    let lists = all(&tree, &|node| node["role"] == shape.list);
    let bounds = &lists.first().expect("the list")["bounds"];
    let at = |index: usize| bounds[index].as_f64().expect("a bound");
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    let blank = ((at(0) + at(2) / 2.0) as f32, (at(1) + at(3) - 8.0) as f32);
    blank
}

/// A right-click at `(x, y)`, and the name of the menu it opens.
fn right_click_at(host: &mut Offscreen, (x, y): (f32, f32)) -> Option<String> {
    let runtime = host.runtime();
    runtime.secondary_down(x, y);
    runtime.secondary_up(x, y);
    idle(host);
    let tree = host.tree();
    let menus = all(&tree, &|node| node["role"] == "menu");
    menus.first().map(|menu| text(menu, "name"))
}

/// A right-click on the entry named `name`, and the name of the menu it opens.
pub(crate) fn right_click(host: &mut Offscreen, shape: Shape, name: &str) -> Option<String> {
    let at = middle_of(host, shape, name);
    right_click_at(host, at)
}

/// A right-click on the view's blank space below its entries, and the name of the menu it
/// opens.
pub(crate) fn right_click_blank(host: &mut Offscreen, shape: Shape) -> Option<String> {
    let at = blank_of(host, shape);
    right_click_at(host, at)
}
