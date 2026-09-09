// SPDX-License-Identifier: MIT

use crate::adapters::{
    DropVolumeQuery, DropVolumes, gio_file_for_location, location_for_file, lookup_drop_volumes,
};
use crate::model::{FileEntry, Location};
use crate::services::{
    CrossVolumeDropStrategy, DropActionInput, DropCommit, DropOverride, TransferKind,
    VolumeRelation, drop_commit, transferable_drop_sources,
};
use crate::ui::browser::ViewState;
use crate::ui::browser::columns::set_cut_path_style;
use crate::ui::browser::paths::{can_remove_location, is_trash_location};
use gtk::prelude::*;
use gtk::{glib, graphene};
use std::cell::{Cell, RefCell};
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

pub(crate) struct PreparedFileDrop {
    pub target: gtk::DropTarget,
    pub state: Rc<FileDropState>,
}

/// Reuses one classification for cursor feedback and the eventual transfer.
pub(crate) struct FileDropState {
    destination: Rc<dyn Fn() -> Option<Location>>,
    last_override: Cell<DropOverride>,
    sources: RefCell<Option<Rc<[Location]>>>,
    classification: RefCell<Option<DropClassification>>,
}

struct DropClassification {
    destination: Location,
    sources: Rc<[Location]>,
    is_noop: bool,
    volumes: DropVolumes,
}

impl DropClassification {
    fn covers(&self, destination: &Location, sources: &Rc<[Location]>) -> bool {
        self.destination == *destination
            && (Rc::ptr_eq(&self.sources, sources) || self.sources == *sources)
    }
}

impl FileDropState {
    fn new(destination: Rc<dyn Fn() -> Option<Location>>) -> Self {
        Self {
            destination,
            last_override: Cell::new(DropOverride::None),
            sources: RefCell::new(None),
            classification: RefCell::new(None),
        }
    }

    pub(crate) fn destination(&self) -> Option<Location> {
        (self.destination)()
    }

    fn reset(&self) {
        self.last_override.set(DropOverride::None);
        self.sources.take();
        self.classification.take();
    }

    fn reload_sources(&self, target: &gtk::DropTarget) {
        let sources = drop_source_locations(target);
        if sources.is_empty() {
            self.reset();
        } else {
            *self.sources.borrow_mut() = Some(sources.into());
        }
    }

    fn sources(&self, target: &gtk::DropTarget) -> Rc<[Location]> {
        if let Some(sources) = self.sources.borrow().clone() {
            return sources;
        }
        let sources: Rc<[Location]> = drop_source_locations(target).into();
        if !sources.is_empty() {
            *self.sources.borrow_mut() = Some(sources.clone());
        }
        sources
    }

    fn sources_matching(&self, target: &gtk::DropTarget, sources: &[Location]) -> Rc<[Location]> {
        let cached = self.sources(target);
        if *cached == *sources {
            cached
        } else {
            sources.into()
        }
    }

    fn describe_volumes(&self) -> String {
        self.classification
            .borrow()
            .as_ref()
            .map_or_else(|| "unclassified".into(), |cached| cached.volumes.describe())
    }

    fn classify(
        self: &Rc<Self>,
        target: &gtk::DropTarget,
        destination: &Location,
        sources: Rc<[Location]>,
    ) -> (VolumeRelation, bool) {
        if sources.is_empty() {
            return (VolumeRelation::Unknown, false);
        }
        if let Some(cached) = self
            .classification
            .borrow()
            .as_ref()
            .filter(|cached| cached.covers(destination, &sources))
        {
            return (cached.volumes.relation(), cached.is_noop);
        }
        let transferable = transferable_drop_sources(destination, &sources);
        let is_noop = transferable.is_empty();
        let query = DropVolumeQuery::new(destination, &transferable);
        let volumes = lookup_drop_volumes(&query, {
            let state = Rc::downgrade(self);
            let target = target.downgrade();
            let destination = destination.clone();
            let sources = sources.clone();
            move |lookup| {
                let Some(state) = state.upgrade() else {
                    return;
                };
                {
                    let mut classification = state.classification.borrow_mut();
                    let Some(cached) = classification
                        .as_mut()
                        .filter(|cached| cached.covers(&destination, &sources))
                    else {
                        return;
                    };
                    cached.volumes = DropVolumes::Ready(lookup);
                }
                if let Some(target) = target.upgrade() {
                    restatus_file_drop(&target, &state);
                }
            }
        });
        let relation = volumes.relation();
        *self.classification.borrow_mut() = Some(DropClassification {
            destination: destination.clone(),
            sources,
            is_noop,
            volumes,
        });
        (relation, is_noop)
    }
}

pub(crate) fn prepare_file_drop_target(
    destination: impl Fn() -> Option<Location> + 'static,
) -> PreparedFileDrop {
    let drop = gtk::DropTarget::new(
        gtk::gdk::FileList::static_type(),
        gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
    );
    drop.set_preload(true);
    let state = Rc::new(FileDropState::new(Rc::new(destination)));
    let state_for_leave = state.clone();
    drop.connect_leave(move |_| state_for_leave.reset());
    let state_for_value = state.clone();
    drop.connect_value_notify(move |target| {
        state_for_value.reload_sources(target);
        restatus_file_drop(target, &state_for_value);
    });
    PreparedFileDrop {
        target: drop,
        state,
    }
}

fn restatus_file_drop(target: &gtk::DropTarget, state: &Rc<FileDropState>) {
    let Some(drop) = target.current_drop() else {
        return;
    };
    let action = file_drop_action(target, state);
    drop.status(target.actions(), action);
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
    let PreparedFileDrop {
        target: drop,
        state: drop_state,
    } = prepare_file_drop_target({
        let destination = destination.clone();
        move || Some(destination.clone())
    });
    let state_for_enter = drop_state.clone();
    drop.connect_enter(move |target, _, _| file_drop_action(target, &state_for_enter));
    let state_for_motion = drop_state.clone();
    drop.connect_motion(move |target, _, _| file_drop_action(target, &state_for_motion));
    let weak = Rc::downgrade(state);
    drop.connect_drop(move |target, value, _, _| {
        let Some(state) = weak.upgrade() else {
            return false;
        };
        transfer_dropped_files(&state, target, value, destination.clone(), &drop_state)
    });
    widget.add_controller(drop);
}

fn transfer_dropped_files(
    state: &Rc<ViewState>,
    target: &gtk::DropTarget,
    value: &glib::Value,
    destination: Location,
    drop_state: &Rc<FileDropState>,
) -> bool {
    let Some(sources) = locations_from_file_list_value(value) else {
        return false;
    };
    if sources.is_empty() {
        return false;
    }
    let commit = file_drop_commit(target, &destination, &sources, drop_state);
    state.commit_file_drop(destination, sources, commit);
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

pub(crate) fn file_drop_action(
    target: &gtk::DropTarget,
    state: &Rc<FileDropState>,
) -> gtk::gdk::DragAction {
    let destination = state.destination();
    let sources = state.sources(target);
    drop_commit_action(classify_file_drop(
        target,
        state,
        destination.as_ref(),
        sources,
        false,
    ))
}

pub(crate) fn file_drop_commit(
    target: &gtk::DropTarget,
    destination: &Location,
    sources: &[Location],
    state: &Rc<FileDropState>,
) -> DropCommit {
    let sources = state.sources_matching(target, sources);
    classify_file_drop(target, state, Some(destination), sources, true)
}

fn drop_source_locations(target: &gtk::DropTarget) -> Vec<Location> {
    target
        .value()
        .as_ref()
        .and_then(locations_from_file_list_value)
        .unwrap_or_default()
}

fn classify_file_drop(
    target: &gtk::DropTarget,
    state: &Rc<FileDropState>,
    destination: Option<&Location>,
    sources: Rc<[Location]>,
    commit: bool,
) -> DropCommit {
    let drop = target.current_drop();
    if drop.is_none() && !commit {
        return DropCommit::Forbidden;
    }
    let override_with = if commit {
        commit_override(target, &state.last_override)
    } else {
        hover_override(target, &state.last_override)
    };
    let Some(destination) = destination else {
        return DropCommit::Forbidden;
    };
    let (relation, is_noop) = state.classify(target, destination, sources.clone());
    let source_actions = drop
        .as_ref()
        .map_or_else(|| target.actions(), |drop| drop.actions());
    let offered = offered_file_actions(target.actions(), source_actions);
    let (strategy, result) =
        preferred_file_drop_commit_for_current_strategy(offered, override_with, relation, is_noop);
    if commit {
        tracing::debug!(
            dest = %destination.diagnostic_path(),
            sources = sources.len(),
            source = sources.first().map(Location::diagnostic_path),
            volume = ?relation,
            volumes = %state.describe_volumes(),
            ?override_with,
            ?strategy,
            event_mods = ?target.current_event_state(),
            keyboard_mods = ?drop_modifier_state(target),
            drop_actions = ?drop.as_ref().map(|drop| drop.actions()),
            ?offered,
            "drop action classified"
        );
    }
    result
}

fn preferred_file_drop_commit_for_current_strategy(
    actions: gtk::gdk::DragAction,
    override_with: DropOverride,
    volume: VolumeRelation,
    is_noop: bool,
) -> (CrossVolumeDropStrategy, DropCommit) {
    let strategy = crate::ui::theme::ThemeManager::shared().cross_volume_drop_strategy();
    let commit = preferred_file_drop_commit(actions, override_with, volume, is_noop, strategy);
    (strategy, commit)
}

/// A compositor's source-side MOVE offer must not prevent Strata's cross-volume copy.
fn offered_file_actions(
    dest_actions: gtk::gdk::DragAction,
    source_actions: gtk::gdk::DragAction,
) -> gtk::gdk::DragAction {
    let mut offered = gtk::gdk::DragAction::empty();
    if dest_actions.contains(gtk::gdk::DragAction::COPY) {
        offered |= gtk::gdk::DragAction::COPY;
    }
    if dest_actions.contains(gtk::gdk::DragAction::MOVE)
        && source_actions.contains(gtk::gdk::DragAction::MOVE)
    {
        offered |= gtk::gdk::DragAction::MOVE;
    }
    offered
}

fn hover_override(target: &gtk::DropTarget, last: &Cell<DropOverride>) -> DropOverride {
    let current = drop_override(target);
    last.set(current);
    current
}

fn commit_override(target: &gtk::DropTarget, last: &Cell<DropOverride>) -> DropOverride {
    let current = drop_override(target);
    if current != DropOverride::None {
        last.set(current);
        current
    } else {
        last.get()
    }
}

fn drop_override(target: &gtk::DropTarget) -> DropOverride {
    let mods = drop_modifier_state(target);
    if mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
        DropOverride::ForceCopy
    } else if mods.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
        DropOverride::ForceMove
    } else {
        DropOverride::None
    }
}

fn drop_modifier_state(target: &gtk::DropTarget) -> gtk::gdk::ModifierType {
    target
        .widget()
        .and_then(|widget| widget.display().default_seat())
        .and_then(|seat| seat.keyboard())
        .map(|keyboard| keyboard.modifier_state())
        .unwrap_or_else(|| target.current_event_state())
}

fn preferred_file_drop_commit(
    actions: gtk::gdk::DragAction,
    override_with: DropOverride,
    volume: VolumeRelation,
    is_noop: bool,
    strategy: CrossVolumeDropStrategy,
) -> DropCommit {
    if is_noop {
        return DropCommit::Forbidden;
    }
    drop_commit(DropActionInput {
        can_copy: actions.contains(gtk::gdk::DragAction::COPY),
        can_move: actions.contains(gtk::gdk::DragAction::MOVE),
        volume,
        override_with,
        strategy,
    })
}

fn drop_commit_action(commit: DropCommit) -> gtk::gdk::DragAction {
    match commit.transfer_kind() {
        TransferKind::Copy => gtk::gdk::DragAction::COPY,
        TransferKind::Move => gtk::gdk::DragAction::MOVE,
        TransferKind::Forbidden => gtk::gdk::DragAction::empty(),
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
