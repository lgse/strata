// SPDX-License-Identifier: MIT

use crate::adapters::gio_file_for_location;
use crate::adapters::location_for_file;
use crate::model::{FileEntry, Location};
use crate::ui::browser::ViewState;
use crate::ui::browser::columns::set_cut_path_style;
use crate::ui::browser::paths::{can_remove_location, is_trash_location};
use gtk::glib;
use gtk::graphene;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
use std::rc::{Rc, Weak};

const DRAG_PROXY_MAX_SIZE: f64 = 72.0;
const DRAG_PROXY_MIN_SIZE: f64 = 32.0;
const DRAG_PROXY_PADDING: f64 = 3.0;
const DRAG_PROXY_STACK_OFFSET: f64 = 5.0;

/// Renders a compact Finder-style file pile and returns its pointer hotspot.
#[expect(
    deprecated,
    reason = "lookup_color is the only way to read custom named CSS colors"
)]
pub(in crate::ui) fn drag_icon_with_count(
    base: &gtk::Widget,
    count: usize,
) -> Option<(gtk::gdk::Texture, i32, i32)> {
    if count <= 1 {
        return None;
    }

    let source_w = f64::from(base.width()).max(1.0);
    let source_h = f64::from(base.height()).max(1.0);
    let source_size = source_w.max(source_h);
    let scale = if source_size < DRAG_PROXY_MIN_SIZE {
        DRAG_PROXY_MIN_SIZE / source_size
    } else {
        (DRAG_PROXY_MAX_SIZE / source_size).min(1.0)
    };
    let icon_w = source_w * scale;
    let icon_h = source_h * scale;
    let front_x = DRAG_PROXY_PADDING;
    let front_y = DRAG_PROXY_PADDING;
    let paintable = gtk::WidgetPaintable::new(Some(base));

    let style = base.style_context();
    let accent = style.lookup_color("theme_accent")?;
    let surface = style
        .lookup_color("theme_surface")
        .or_else(|| style.lookup_color("theme_bg"))?;
    let text = style.lookup_color("theme_text")?;
    let badge_text = contrasting_badge_text(&accent, &text, &surface);

    let label = count.to_string();
    let layout = base.create_pango_layout(Some(&label));
    if let Some(mut font) = layout.font_description() {
        font.set_weight(gtk::pango::Weight::Semibold);
        layout.set_font_description(Some(&font));
    }
    let (ink, _) = layout.pixel_extents();
    let (badge_w, badge_h) = badge_dimensions(f64::from(ink.width()), f64::from(ink.height()));
    let badge_x = front_x + icon_w - badge_w * 0.4;
    let badge_y = front_y + icon_h - badge_h * 0.4;
    let rear_extent = DRAG_PROXY_STACK_OFFSET + DRAG_PROXY_PADDING;
    let canvas_w = (badge_x + badge_w).max(front_x + icon_w) + DRAG_PROXY_PADDING;
    let canvas_h = (badge_y + badge_h).max(front_y + icon_h) + DRAG_PROXY_PADDING;
    let canvas_w = canvas_w.max(icon_w + rear_extent + DRAG_PROXY_PADDING);
    let canvas_h = canvas_h.max(icon_h + rear_extent + DRAG_PROXY_PADDING);

    let snapshot = gtk::Snapshot::new();
    let transparent = gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0);
    snapshot.append_color(
        &transparent,
        &graphene::Rect::new(0.0, 0.0, canvas_w as f32, canvas_h as f32),
    );

    for (offset, opacity) in [(DRAG_PROXY_STACK_OFFSET, 0.32), (2.5, 0.6)] {
        snapshot.push_opacity(opacity);
        snapshot.save();
        snapshot.translate(&graphene::Point::new(
            (front_x + offset) as f32,
            (front_y + offset) as f32,
        ));
        paintable.snapshot(&snapshot, icon_w, icon_h);
        snapshot.restore();
        snapshot.pop();
    }

    let mut shadow_color = text;
    shadow_color.set_alpha(0.34);
    snapshot.push_shadow(&[gtk::gsk::Shadow::new(shadow_color, 0.0, 1.0, 3.0)]);
    snapshot.save();
    snapshot.translate(&graphene::Point::new(front_x as f32, front_y as f32));
    paintable.snapshot(&snapshot, icon_w, icon_h);
    snapshot.restore();
    snapshot.pop();

    let badge_rect = gtk::gsk::RoundedRect::from_rect(
        graphene::Rect::new(
            badge_x as f32,
            badge_y as f32,
            badge_w as f32,
            badge_h as f32,
        ),
        (badge_h / 2.0) as f32,
    );
    snapshot.push_rounded_clip(&badge_rect);
    snapshot.append_color(&accent, badge_rect.bounds());
    snapshot.pop();
    snapshot.append_border(
        &badge_rect,
        &[1.0; 4],
        &[surface, surface, surface, surface],
    );
    let tx = badge_x + (badge_w - f64::from(ink.width())) / 2.0 - f64::from(ink.x());
    let ty = badge_y + (badge_h - f64::from(ink.height())) / 2.0 - f64::from(ink.y());
    snapshot.save();
    snapshot.translate(&graphene::Point::new(tx as f32, ty as f32));
    snapshot.append_layout(&layout, &badge_text);
    snapshot.restore();

    let renderer = base.native().and_then(|native| native.renderer())?;
    let node = snapshot.to_node()?;
    let texture = renderer.render_texture(&node, None);
    Some((
        texture,
        (front_x + icon_w / 2.0).round() as i32,
        (front_y + icon_h / 2.0).round() as i32,
    ))
}

fn badge_dimensions(text_width: f64, text_height: f64) -> (f64, f64) {
    let height = (text_height + 6.0).max(20.0);
    ((text_width + 10.0).max(height), height)
}

fn contrasting_badge_text(
    fill: &gtk::gdk::RGBA,
    text: &gtk::gdk::RGBA,
    surface: &gtk::gdk::RGBA,
) -> gtk::gdk::RGBA {
    if contrast_ratio(fill, text) >= contrast_ratio(fill, surface) {
        *text
    } else {
        *surface
    }
}

fn contrast_ratio(first: &gtk::gdk::RGBA, second: &gtk::gdk::RGBA) -> f64 {
    let first = relative_luminance(first);
    let second = relative_luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn relative_luminance(color: &gtk::gdk::RGBA) -> f64 {
    let linear = |channel: f32| {
        let channel = f64::from(channel);
        if channel <= 0.03928 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.red()) + 0.7152 * linear(color.green()) + 0.0722 * linear(color.blue())
}

pub(super) fn install_directory_drop_target(
    state: &Rc<ViewState>,
    widget: &impl IsA<gtk::Widget>,
    destination: Location,
) {
    if is_trash_location(&destination) {
        return;
    }
    widget.add_css_class("file-drop-zone");
    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
    );
    drop.connect_enter(|target, _, _| file_drop_action(target));
    drop.connect_motion(|target, _, _| file_drop_action(target));
    let weak = Rc::downgrade(state);
    drop.connect_drop(move |target, value, _, _| {
        let Some(state) = weak.upgrade() else {
            return false;
        };
        transfer_dropped_files(&state, target, value, destination.clone())
    });
    widget.add_controller(drop);
}

fn transfer_dropped_files(
    state: &Rc<ViewState>,
    target: &gtk::DropTarget,
    value: &glib::Value,
    destination: Location,
) -> bool {
    let Some(sources) = locations_from_file_list_value(value) else {
        return false;
    };
    if sources.is_empty() {
        return false;
    }
    let move_sources = file_drop_action(target) == gtk::gdk::DragAction::MOVE;
    state.start_transfer(destination, sources, move_sources);
    true
}

pub(crate) fn drag_actions_for_modifiers(
    modifiers: gtk::gdk::ModifierType,
) -> gtk::gdk::DragAction {
    if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
        gtk::gdk::DragAction::COPY
    } else if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
        gtk::gdk::DragAction::MOVE
    } else {
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE
    }
}

pub(crate) fn file_drop_action(target: &gtk::DropTarget) -> gtk::gdk::DragAction {
    let Some(drop) = target.current_drop() else {
        return gtk::gdk::DragAction::empty();
    };
    let selected = drop.drag().map(|drag| drag.selected_action());
    selected
        .filter(|action| !action.is_empty())
        .unwrap_or_else(|| preferred_file_drop_action(drop.actions(), drop.drag().is_some()))
}

fn preferred_file_drop_action(actions: gtk::gdk::DragAction, local: bool) -> gtk::gdk::DragAction {
    if actions.contains(gtk::gdk::DragAction::MOVE)
        && (local || !actions.contains(gtk::gdk::DragAction::COPY))
    {
        gtk::gdk::DragAction::MOVE
    } else if actions.contains(gtk::gdk::DragAction::COPY) {
        gtk::gdk::DragAction::COPY
    } else {
        gtk::gdk::DragAction::empty()
    }
}

pub(crate) fn locations_from_file_list_value(value: &glib::Value) -> Option<Vec<Location>> {
    let files = value.get::<gtk::gdk::FileList>().ok()?;
    let locations = files
        .files()
        .iter()
        .filter_map(location_for_file)
        .collect::<Vec<_>>();
    (!locations.is_empty()).then_some(locations)
}

pub(in crate::ui) fn file_drag_content(entries: &[FileEntry]) -> Option<gtk::gdk::ContentProvider> {
    let files = entries
        .iter()
        .map(|entry| gio_file_for_location(&entry.location))
        .collect::<Vec<_>>();
    if files.is_empty() {
        return None;
    }
    let file_list =
        gtk::gdk::ContentProvider::for_value(&gtk::gdk::FileList::from_array(&files).to_value());
    let uri_list = files
        .iter()
        .map(|file| file.uri())
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n";
    let uri_list = gtk::gdk::ContentProvider::for_bytes(
        "text/uri-list",
        &glib::Bytes::from_owned(uri_list.into_bytes()),
    );
    Some(gtk::gdk::ContentProvider::new_union(&[file_list, uri_list]))
}

pub(super) fn copy_locations(entries: &[FileEntry]) {
    let text = entries
        .iter()
        .map(|entry| copy_path_text(&entry.location, entry.is_directory()))
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(&text);
    }
}

pub(super) fn copy_names(entries: &[FileEntry]) {
    let text = entries
        .iter()
        .map(|entry| entry.display_name.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(&text);
    }
}

pub(super) fn copy_path_text(location: &Location, is_directory: bool) -> String {
    match location.native_path() {
        Some(path) => {
            let mut path = shell_escape_path(path);
            if is_directory && !path.ends_with(std::path::MAIN_SEPARATOR) {
                path.push(std::path::MAIN_SEPARATOR);
            }
            path
        }
        None => location.display_path(),
    }
}

fn shell_escape_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if path.contains('\n') {
        return format!("'{}'", path.replace('\'', "'\\''"));
    }

    let mut escaped = String::new();
    for c in path.chars() {
        if needs_shell_escape(c) {
            escaped.push('\\');
            escaped.push(c);
        } else {
            escaped.push(c);
        }
    }
    escaped
}

fn needs_shell_escape(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '"' | '\''
                | '\\'
                | '$'
                | '`'
                | '!'
                | '#'
                | '&'
                | '*'
                | ';'
                | '<'
                | '>'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '('
                | ')'
                | '|'
                | '~'
        )
}

// Process-wide cut intent shared by every window. The GDK clipboard only
// carries a `FileList` with no cut marker, so this thread-local (GTK stays on
// the main thread) is the source of truth for both paste behavior and styling.
thread_local! {
    static SHARED_CUT_LOCATIONS: RefCell<Vec<Location>> = const { RefCell::new(Vec::new()) };
    static CUT_VIEWS: RefCell<Vec<Weak<ViewState>>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn register_cut_view(state: &Rc<ViewState>) {
    CUT_VIEWS.with(|views| views.borrow_mut().push(Rc::downgrade(state)));
    state.refresh_cut_rows();
}

fn refresh_cut_views() {
    let views = CUT_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let live = views.iter().filter_map(Weak::upgrade).collect::<Vec<_>>();
        views.retain(|view| view.strong_count() > 0);
        live
    });
    for view in views {
        view.refresh_cut_rows();
    }
}

pub(super) fn shared_cut_locations() -> Vec<Location> {
    SHARED_CUT_LOCATIONS.with(|cut| cut.borrow().clone())
}

fn set_shared_cut(locations: &[Location]) {
    SHARED_CUT_LOCATIONS.with(|cut| cut.replace(locations.to_vec()));
    refresh_cut_views();
}

fn clear_shared_cut() {
    SHARED_CUT_LOCATIONS.with(|cut| cut.borrow_mut().clear());
    refresh_cut_views();
}

fn retain_shared_untransferred(transferred: &[Location]) {
    SHARED_CUT_LOCATIONS.with(|cut| retain_untransferred(&mut cut.borrow_mut(), transferred));
    refresh_cut_views();
}

fn is_cut_match(sources: &[Location]) -> bool {
    same_locations(sources, &shared_cut_locations())
}

fn set_files_clipboard(entries: &[FileEntry]) -> bool {
    set_location_files_clipboard(
        &entries
            .iter()
            .map(|entry| entry.location.clone())
            .collect::<Vec<_>>(),
    )
}

fn set_location_files_clipboard(locations: &[Location]) -> bool {
    let files = locations
        .iter()
        .map(gio_file_for_location)
        .collect::<Vec<_>>();
    if files.is_empty() {
        return false;
    }
    gtk::gdk::Display::default().is_some_and(|display| {
        display
            .clipboard()
            .set_content(Some(&gtk::gdk::ContentProvider::for_value(
                &gtk::gdk::FileList::from_array(&files).to_value(),
            )))
            .is_ok()
    })
}

/// Location equality that also accepts GIO-level equivalence (URI
/// normalization, `file://` vs native path for the same file). Mounts such as
/// NFS can round-trip through the clipboard with a different but equivalent
/// representation, and strict `PathBuf` equality alone would degrade a cut to
/// a copy.
pub(super) fn locations_equal(left: &Location, right: &Location) -> bool {
    left == right || gio_file_for_location(left).equal(&gio_file_for_location(right))
}

fn same_locations(left: &[Location], right: &[Location]) -> bool {
    if left.is_empty() || left.len() != right.len() {
        return false;
    }
    let left_set: HashSet<_> = left.iter().collect();
    let right_set: HashSet<_> = right.iter().collect();
    if left_set.len() == right_set.len() && left_set == right_set {
        return true;
    }
    let mut used = vec![false; right.len()];
    left.iter().all(|location| {
        let Some((index, _)) = right
            .iter()
            .enumerate()
            .find(|(index, candidate)| !used[*index] && locations_equal(location, candidate))
        else {
            return false;
        };
        used[index] = true;
        true
    })
}

fn retain_untransferred(cut: &mut Vec<Location>, transferred: &[Location]) {
    cut.retain(|location| {
        !transferred
            .iter()
            .any(|moved| locations_equal(location, moved))
    });
}

impl ViewState {
    pub(super) fn copy_entries(&self, entries: &[FileEntry]) {
        if set_files_clipboard(entries) {
            self.clear_cut();
        }
    }

    pub(super) fn cut_entries(&self, entries: &[FileEntry]) -> bool {
        if entries
            .iter()
            .any(|entry| !can_remove_location(&entry.location))
        {
            return false;
        }
        if set_files_clipboard(entries) {
            let locations: Vec<Location> =
                entries.iter().map(|entry| entry.location.clone()).collect();
            set_shared_cut(&locations);
            return true;
        }
        false
    }

    fn clear_cut(&self) {
        clear_shared_cut();
    }

    pub(super) fn complete_cut_transfer(&self, transferred: &[Location]) {
        retain_shared_untransferred(transferred);
        let remaining = shared_cut_locations();
        if remaining.is_empty() {
            if let Some(display) = gtk::gdk::Display::default() {
                let _result = display
                    .clipboard()
                    .set_content(None::<&gtk::gdk::ContentProvider>);
            }
        } else {
            let _set = set_location_files_clipboard(&remaining);
        }
    }

    fn refresh_cut_rows(&self) {
        let cut = shared_cut_locations();
        self.mode_views.borrow().set_cut_locations(&cut);
        let cut_lookup: HashSet<_> = cut.iter().collect();
        for (depth, column) in self.columns.borrow().iter().enumerate() {
            column.bound_rows.borrow_mut().retain(|bound| {
                let (Some(item), Some(row)) = (bound.item.upgrade(), bound.row.upgrade()) else {
                    return false;
                };
                let is_cut = column
                    .map
                    .source_position(item.position())
                    .and_then(|position| self.browser.entry_at(depth, position))
                    .is_some_and(|entry| cut_lookup.contains(&entry.location));
                set_cut_path_style(&row, is_cut);
                true
            });
        }
    }

    pub(super) fn paste_into(self: &Rc<Self>, destination: Location) {
        if is_trash_location(&destination) {
            return;
        }
        let Some(display) = gtk::gdk::Display::default() else {
            return;
        };
        let clipboard = display.clipboard();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = clipboard
                .read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT)
                .await;
            let files = match result {
                Ok(value) => match value.get::<gtk::gdk::FileList>() {
                    Ok(files) => files.files(),
                    Err(_) => return,
                },
                Err(_) => return,
            };
            let sources = files
                .into_iter()
                .filter_map(|file| location_for_file(&file))
                .collect::<Vec<_>>();
            if let Some(state) = weak.upgrade() {
                let move_sources = is_cut_match(&sources);
                state.start_transfer(destination, sources, move_sources);
            }
        });
    }
}

#[cfg(test)]
mod tests;
