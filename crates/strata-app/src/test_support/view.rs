// SPDX-License-Identifier: MIT

//! What the single-pane views' tests share: a window opened in a saved mode, keys fed to it,
//! and the view read from the accessibility tree as the QA harness reads it.

use std::path::{Path, PathBuf};

use rune::{Key, KeyEvent, Mods, WheelEvent};
use serde_json::Value;
use strata_components::{collection::element::row_key, contract::EntryId};
use strata_core::model::Location;

use super::{PATIENCE, hosted_after, hosted_on, send_key};
use crate::{App, host::offscreen::Offscreen, slots::Entry};

fn settings_file() -> PathBuf {
    strata_core::storage::config_directory().join("settings.toml")
}

/// Saves `mode` as the view to show, and `more` after it: lines of `settings.toml`.
fn save_mode(mode: &str, more: &str) {
    let file = settings_file();
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("settings folder");
    let saved =
        format!("mode = \"theme\"\ntheme = \"azure-glow\"\nbrowser_mode = \"{mode}\"\n{more}");
    std::fs::write(file, saved).expect("settings");
}

/// Runs the test `name` of `module` in a window whose saved `browser_mode` is `mode`.
pub(crate) fn hosted_in(module: &str, mode: &'static str, name: &str, run: impl FnOnce(Offscreen)) {
    hosted_with(module, mode, String::new(), name, run);
}

/// [`hosted_in`], with `more` saved too: lines of `settings.toml`.
pub(crate) fn hosted_with(
    module: &str,
    mode: &'static str,
    more: String,
    name: &str,
    run: impl FnOnce(Offscreen),
) {
    hosted_after(module, name, move || save_mode(mode, &more), run);
}

/// A test on a window made of the table `parts`, with `mode` saved as the view to show.
pub(crate) fn hosted_over_in(
    (module, name): (&str, &str),
    mode: &'static str,
    parts: fn() -> Vec<Entry<App>>,
    run: impl FnOnce(Offscreen),
) {
    let host = |ui, settings| Offscreen::with_parts(parts().leak(), ui, settings);
    hosted_on((module, name), move || save_mode(mode, ""), host, run);
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
    /// The list itself has the keyboard.
    pub(crate) keyboard: bool,
    /// The labels the list draws outside its entries: a status in place of entries.
    pub(crate) texts: Vec<String>,
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

/// The labels under `node` that are not in an entry: what the list draws besides its rows.
fn outside_entries(node: &Value, shape: Shape) -> Vec<&Value> {
    let mut found = Vec::new();
    for child in node["children"].as_array().map_or(&[][..], Vec::as_slice) {
        if child["role"] == shape.entry && keyed_as_entry(child) {
            continue;
        }
        if child["role"] == "label" {
            found.push(child);
        }
        found.extend(outside_entries(child, shape));
    }
    found
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
        keyboard: has_state(list, "focused"),
        texts: outside_entries(list, shape)
            .iter()
            .map(|label| text(label, "name"))
            .collect(),
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

/// How many entries `folder` lists with hidden files hidden: what a view shows once it has
/// it all.
fn shown_in(folder: &Path) -> usize {
    let entries = std::fs::read_dir(folder).expect("a folder to read");
    entries
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .count()
}

/// Opens `folder` and waits for all its entries: a listing can arrive over several turns,
/// so a first look may show only part of it.
pub(crate) fn open(host: &mut Offscreen, shape: Shape, folder: &Path) -> Seen {
    host.app().open(Location::local(folder));
    let count = shown_in(folder);
    wait(host, shape, "the folder's whole listing", |seen| {
        seen.entries.len() == count
    });
    idle(host);
    seen(host, shape)
}

/// Feeds the host a key with its modifiers held, as a keyboard sends one.
pub(crate) fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
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
    open_menu(host)
}

/// The names of the entries drawn now, in every list, in the tree's order: a list builds the
/// rows in and near its viewport.
pub(crate) fn drawn(host: &Offscreen) -> Vec<String> {
    let tree = host.tree();
    let entries = all(&tree, &keyed_as_entry);
    entries.iter().map(|entry| text(entry, "name")).collect()
}

/// Where the first entry drawn has its top: for telling that a scroll happened, never for
/// saying where anything should be.
pub(crate) fn top_of_first(host: &Offscreen) -> Option<f64> {
    let tree = host.tree();
    let entries = all(&tree, &keyed_as_entry);
    entries.first()?["bounds"][1].as_f64()
}

/// Where the first entry is drawn now: its centre, to point a wheel at.
fn first_entry_centre(host: &Offscreen) -> (f32, f32) {
    let tree = host.tree();
    let entries = all(&tree, &keyed_as_entry);
    let bounds = &entries.first().expect("an entry")["bounds"];
    let at = |index: usize| bounds[index].as_f64().expect("a bound");
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    let centre = ((at(0) + at(2) / 2.0) as f32, (at(1) + at(3) / 2.0) as f32);
    centre
}

/// Turns the wheel `turns` times by `dy` points, over where the first entry is drawn now.
pub(crate) fn wheel(host: &mut Offscreen, dy: f32, turns: usize) {
    let (x, y) = first_entry_centre(host);
    for _ in 0..turns {
        host.runtime().wheel(x, y, dy);
        idle(host);
    }
}

/// One notch of the wheel by `dy` points with `mods` held, over where the first entry is
/// drawn now, offered to the elements' wheel handlers as a window host offers it.
pub(crate) fn wheel_held(host: &mut Offscreen, dy: f32, mods: Mods) {
    let (x, y) = first_entry_centre(host);
    host.runtime()
        .wheel_event(WheelEvent::new(0.0, dy).at(x, y).mods(mods));
    idle(host);
}

/// The name of the menu that is open: what it acts on (`menus.rs`).
pub(crate) fn open_menu(host: &Offscreen) -> Option<String> {
    let tree = host.tree();
    let menus = all(&tree, &|node| node["role"] == "menu");
    menus.first().map(|menu| text(menu, "name"))
}

/// A double click on the entry named `name`, as a pointer opens it.
pub(crate) fn double_click(host: &mut Offscreen, shape: Shape, name: &str) {
    let (x, y) = middle_of(host, shape, name);
    let runtime = host.runtime();
    runtime.pointer_move(x, y);
    for _ in 0..2 {
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
    }
    idle(host);
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

/// A click on the open menu's item named `name`.
pub(crate) fn choose(host: &mut Offscreen, name: &str) {
    let tree = host.tree();
    let items = all(&tree, &|node| {
        node["role"]
            .as_str()
            .is_some_and(|role| role.starts_with("menu-item"))
            && node["name"] == name
    });
    let bounds = &items.first().expect("the menu item")["bounds"];
    let at = |index: usize| bounds[index].as_f64().expect("a bound");
    #[expect(clippy::cast_possible_truncation, reason = "window points")]
    let (x, y) = ((at(0) + at(2) / 2.0) as f32, (at(1) + at(3) / 2.0) as f32);
    let runtime = host.runtime();
    runtime.pointer_move(x, y);
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    idle(host);
}

/// The opacity, as drawn, of the placeholder icon in the entry named `name`: C25's glyph under
/// the element the entry is named on.
pub(crate) fn icon_opacity(host: &mut Offscreen, name: &str) -> Option<f32> {
    let elements = host.runtime().elements();
    let entry = elements
        .iter()
        .find(|e| !e.exiting && e.label.as_deref() == Some(name))?;
    elements
        .iter()
        .filter(|e| !e.exiting && e.key.as_deref() == Some(strata_components::thumbnails::GLYPH))
        .find(|e| e.path.starts_with(&entry.path))
        .map(|e| e.opacity)
}
