// SPDX-License-Identifier: MIT

//! Preview drawer part (`A15`, `Place::Drawer`): mounts C9's [`PreviewDrawer`] around the shown
//! file view, routes Space / Escape / Right / `F8` / `preview-panel`, follows `Browser` focus
//! and selection events, and hosts read-only text and code previews (`C10a` / `C10b`).
//!
//! Other preview packages plug into [`viewer::Viewer`] and [`load`]:
//! - `A16` adds PDF, media, 3D model, and GPS route viewers.
//! - `A13` adds archive tree previews and archive password prompts.

use std::{cell::RefCell, path::Path, rc::Rc};

use rune::{El, Tx};
use strata_components::preview_drawer::{
    DocumentView, DrawerIntent, DrawerRules, DrawerState, HeaderInfo, HostedPreview, PreviewDrawer,
};
use strata_core::{
    i18n::tr,
    model::{EntryKind, FileEntry, Location, MetadataValue},
};
use strata_fs::services::{
    PreviewContent,
    file_activity::{self, Notice},
};

mod archive;
mod document;
mod follow;
mod image;
mod keys;
mod labels;
mod layout;
mod load;
mod raw;
mod session;
mod sheet;
pub(crate) mod viewer;

#[cfg(test)]
mod tests;

pub(crate) use archive::{ARCHIVE_KEY, PROMPT_KEY};
pub(crate) use follow::{
    follow_active_column, follow_filter_close, follow_filter_pick, note_click_opening,
};
use load::Loader;
use viewer::Viewer;

use crate::{
    App,
    commands::{Action, Holds},
    shell,
    slots::{Frame, Listener, Part, Place, Setup},
};

pub(crate) struct Preview {
    drawer: DrawerState,
    viewer: Viewer,
    raw: raw::RawSection,
    header: HeaderInfo,
    loader: Loader,
    document_view: DocumentView,
    enabled: bool,
    reserve_columns: bool,
    dismissed: bool,
    child_pane: bool,
    current: Option<FileEntry>,
    current_depth: Option<usize>,
    header_source: Option<(FileEntry, Option<String>)>,
    content_focus: ContentFocus,
    tab_shown: bool,
    click_opening: Option<ClickOpening>,
    focus_timer: Rc<RefCell<Option<glib::SourceId>>>,
    released_from: Option<Option<usize>>,
    operations: Option<file_activity::Subscription>,
    last_split_width: Option<f32>,
    was_in_band: bool,
}

/// The folder a click in Columns is opening, at `depth`, until its column has loaded.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ClickOpening {
    pub(super) depth: usize,
    pub(super) folder: Location,
}

/// Whether a `No visual preview` card has the keyboard: asked for until it shows, then owned
/// until the keyboard leaves it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ContentFocus {
    #[default]
    None,
    Pending,
    Owned,
}

impl Preview {
    pub(super) fn reserves_column_space(&self, app: &App) -> bool {
        self.reserve_columns && session::browsing_columns(app)
    }

    pub(super) fn reserves_empty_preview(&self, app: &App) -> bool {
        self.enabled
            && !self.child_pane
            && (session::browsing_columns(app) || session::browsing_icons(app))
    }

    pub(super) fn preview_present(&self, app: &App) -> bool {
        self.current.is_some()
            || self.reserves_empty_preview(app)
            || self.reserves_column_space(app)
    }

    /// Follows the keyboard into and out of a `No visual preview` card, which takes it only
    /// while it owns it (GTK's `content.set_focusable(true)` and focus-leave controller,
    /// `src/ui/preview/keyboard.rs`). Whether the card stops taking it, so the view redraws.
    fn sync_message_focus(&mut self, tx: &Tx) -> bool {
        let on_card = tx.focused() == Some(viewer::MESSAGE_KEY);
        match (self.content_focus, on_card) {
            (ContentFocus::Pending, true) => {
                self.content_focus = ContentFocus::Owned;
                false
            }
            (ContentFocus::Owned, false) => {
                self.content_focus = ContentFocus::None;
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn media_backend(&self) -> strata_preview::sandbox::MediaPreviewBackend {
        self.loader.media_backend()
    }
}

pub(crate) fn refresh_header(app: &mut App) {
    if let Some((entry, content_type)) = &app.preview.header_source {
        app.preview.header = load::identity_for(&app.look, entry, content_type.as_deref());
    }
}

pub(crate) fn sync_preferences(app: &mut App) {
    let prefs = shell::preferences(app);
    let wrap = prefs.preview_text_wrap;
    let backend = load::effective_media_backend(prefs);
    app.preview.viewer.set_wrap(wrap);
    app.preview.loader.set_media_backend(backend);
    refresh_header(app);
}

impl Part for Preview {
    const PLACE: Place<App> = Place::Drawer(wrap);
    const COMMANDS: &'static [Action<App>] = keys::COMMANDS;

    fn new(setup: &Setup<'_>) -> Self {
        let mut drawer = DrawerState::new();
        drawer.reserve(setup.settings.browser_mode == "columns");
        let loader = Loader::new(setup.later.clone(), setup.ids.clone());
        loader.set_media_backend(load::effective_media_backend(setup.settings));
        Self {
            drawer,
            viewer: Viewer::Empty,
            raw: raw::RawSection::default(),
            header: HeaderInfo::default(),
            loader,
            document_view: DocumentView::Rendered,
            enabled: false,
            reserve_columns: true,
            dismissed: false,
            child_pane: false,
            current: None,
            current_depth: None,
            header_source: None,
            content_focus: ContentFocus::None,
            tab_shown: false,
            click_opening: None,
            focus_timer: Rc::default(),
            released_from: None,
            operations: None,
            last_split_width: None,
            was_in_band: false,
        }
    }

    fn keyboard(app: &App) -> Option<Holds> {
        (app.preview.drawer.is_find_focused() || app.preview.viewer.is_prompt_focused())
            .then_some(Holds::Field)
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(follow::hear))
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        ensure_operations_watch(app);
        app.preview.loader.sync_theme(&app.look);
        let blocks = app.preview.loader.poll_blocks();
        app.columns
            .refresh_mirror_preferences(shell::preferences(app));
        let lit = session::browsing_columns(app) && app.columns.last_edge_lit();
        let lit_changed = app.preview.drawer.line_lit() != lit;
        app.preview.drawer.light_line(lit);
        let banding = session::in_band(app);
        let band_ended = std::mem::replace(&mut app.preview.was_in_band, banding) && !banding;
        let band_synced = if band_ended && app.preview.enabled {
            let (depth, entry) = session::selected_entry(app);
            follow::follow_focus(app, depth.unwrap_or(0), entry, tx)
        } else {
            false
        };
        let msg_focus = app.preview.sync_message_focus(tx);
        let viewer = app.preview.viewer.poll(clock, &app.look, tx);
        let archive = viewer::tick_archive_if_active(app, tx);
        blocks | viewer | archive | lit_changed | band_synced | msg_focus
    }

    fn switched(app: &mut App, tx: &mut Tx) {
        let tab_shown = std::mem::take(&mut app.preview.tab_shown);
        session::sync_reservation(app);
        if !app.preview.enabled && !app.preview.reserves_column_space(app) {
            app.preview.drawer.close(&app.look, tx);
            return;
        }
        if app.preview.enabled && app.preview.current.is_none() {
            session::show_empty_slot_after_switch(app, tab_shown, tx);
        }
    }

    fn tab_shown(app: &mut App, _tx: &mut Tx) {
        app.preview.tab_shown = true;
        ensure_operations_watch(app);
        sync_preferences(app);
        let prefs = shell::preferences(app);
        app.columns.refresh_mirror_preferences(prefs);
        session::sync_reservation(app);
    }
}

pub(crate) fn current_entry(app: &App) -> Option<&FileEntry> {
    app.preview.current.as_ref()
}

fn wrap(app: &App, files: El, frame: &mut Frame<'_, App>) -> El {
    let files_key = shell::view_focus(app).unwrap_or("columns");
    let tenxer = shell::preferences(app).tenxer_mode;
    let mut drawer =
        PreviewDrawer::new(&app.preview.drawer, |app: &mut App| &mut app.preview.drawer)
            .arrangement(layout::arrangement(app))
            .rules(DrawerRules::STRATA)
            .labels(labels::drawer())
            .files_key(files_key)
            .find_keys(keys::find_keys_in(tenxer))
            .hosted(&app.preview.viewer, |app: &mut App| &mut app.preview.viewer)
            .on_background_drag(on_background_drag)
            .on_intent(on_intent);

    if app.preview.current.is_some() {
        drawer = drawer
            .header(app.preview.header.clone())
            .drag_source(crate::dnd::binder(app), crate::dnd::preview_drag_source());
        let body = app.preview.viewer.build(app, frame.cx).unwrap_or_else(|| {
            // Keep the slot mounted while a fresh load waits on its 120 ms spinner delay.
            rune::el().key("preview-awaiting").w_full().h_full()
        });
        drawer = drawer.content(app.preview.raw.wrap_body(&app.look, body));
    } else if app.preview.reserves_empty_preview(app) {
        drawer = drawer
            .header(HeaderInfo::new(tr("Preview")))
            .placeholder(tr("No preview for this selection"));
    }

    drawer
        .build(files, &app.look, frame.cx)
        .on_layout_tx(layout::on_split_layout)
}

fn on_background_drag(app: &mut App, d: &mut rune::Drag, clears: bool, tx: &mut Tx) {
    if session::browsing_columns(app) {
        crate::columns::view(app).origin_drag(app, (d, clears), tx);
    } else if session::browsing_icons(app) {
        crate::icons::origin_drag(app, d, clears, tx);
    } else {
        crate::list::origin_drag(app, d, clears, tx);
    }
    if needs_post_drag_settle(d.phase, clears) {
        shell::publish(app);
        shell::settle(app, tx);
    }
}

fn needs_post_drag_settle(phase: rune::Phase, clears: bool) -> bool {
    if phase == rune::Phase::Up {
        return true;
    }
    phase == rune::Phase::Down && clears
}

fn on_intent(app: &mut App, intent: DrawerIntent, tx: &mut Tx) {
    match intent {
        DrawerIntent::Closed => {
            app.preview.dismissed = true;
            app.preview.enabled = false;
            app.preview.child_pane = false;
            session::clear_target(app, tx);
        }
        DrawerIntent::Open => open_current(app),
        DrawerIntent::ShowView(view) => show_document_view(app, view, tx),
        DrawerIntent::SetWrap(wrap) => {
            app.preview.viewer.set_wrap(wrap);
            shell::saved(app).change(|settings| settings.preview_text_wrap = wrap);
        }
        DrawerIntent::Suspend(suspended) => on_suspend(app, suspended, tx),
        DrawerIntent::EmptySlotPressed => {
            app.columns.clear_selection();
            tx.focus(crate::columns::VIEW, false);
        }
        _ => {}
    }
}

fn show_document_view(app: &mut App, view: DocumentView, tx: &mut Tx) {
    if view == DocumentView::Rendered && app.preview.viewer.take_render_pending() {
        if let Some(entry) = app.preview.current.clone() {
            load::reload_rendered(app, entry);
        }
        return;
    }
    if app.preview.viewer.show_view(view) {
        app.preview.document_view = view;
        if app.preview.drawer.is_finding() {
            let query = app.preview.drawer.find_query().to_owned();
            app.preview.viewer.find(&query);
            tx.wake_after(std::time::Duration::ZERO);
        }
    }
}

fn on_suspend(app: &mut App, suspended: bool, tx: &mut Tx) {
    if suspended {
        if app.preview.loader.is_active() {
            app.preview.loader.cancel();
            app.preview.loader.defer_on_resume = true;
        }
        return;
    }
    if session::browsing_columns(app) {
        app.columns.reveal_active(&app.look, tx);
    }
    let reload = std::mem::replace(&mut app.preview.loader.defer_on_resume, false);
    if reload
        && app.preview.enabled
        && let Some(entry) = app.preview.current.clone()
    {
        load::start(app, entry);
    }
}

pub(super) fn open_current(app: &mut App) {
    if let Some(location) = app
        .preview
        .current
        .as_ref()
        .map(|entry| entry.location.clone())
    {
        app.browser.open_location(location);
    }
}

/// Whether the Appearance popover's "Preview panel" toggle is pressed.
pub(crate) fn panel_pressed(app: &App) -> bool {
    app.preview.enabled || app.preview.reserves_column_space(app)
}

/// Closes an open preview on Escape (`Stage::Dismissal`), returning whether one closed.
pub(crate) fn dismiss_escape(app: &mut App, tx: &mut Tx) -> bool {
    if !app.preview.enabled {
        return false;
    }
    session::close(app, tx);
    if let Some(view) = shell::view_focus(app) {
        tx.focus(view, true);
    }
    true
}

/// Toggles the preview for `entry` at `depth` (called by C18's `FilterIntent::Preview`).
pub(crate) fn toggle_entry(
    app: &mut App,
    entry: Option<FileEntry>,
    depth: Option<usize>,
    tx: &mut Tx,
) {
    session::toggle(app, entry, depth, tx);
}

/// Clears the preview's current target when navigating to a folder from the search palette.
pub(crate) fn clear_search_target(app: &mut App, tx: &mut Tx) {
    session::clear_target(app, tx);
}

/// Opens the preview drawer for a file chosen in the search palette when `search_open_files_directly` is off.
pub(crate) fn show_search_file(app: &mut App, path: &Path, name: String, tx: &mut Tx) {
    let native_name = path.file_name().unwrap_or_default().to_os_string();
    let entry = FileEntry {
        location: Location::local(path.to_path_buf()),
        native_name,
        thumbnail_path: None,
        display_name: name,
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden: false,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        recent_uri: None,
    };
    session::show(app, entry, app.browser.active_depth(), tx);
}

/// Opens or updates the preview drawer on `entry` at `depth` without closing an already-open
/// drawer, and hands it keyboard focus (`enter_preview` in `src/ui/window/keyboard/preview.rs`).
pub(crate) fn show_entry(app: &mut App, entry: FileEntry, depth: Option<usize>, tx: &mut Tx) {
    keys::enter_entry(app, entry, depth, tx);
}

/// Whether single clicks open files in the currently shown view.
pub(crate) fn one_click_files_in_shown_view(app: &App) -> bool {
    let settings = shell::preferences(app);
    let modes = if session::browsing_columns(app) {
        crate::clicks::columns(settings)
    } else if session::browsing_icons(app) {
        crate::clicks::icons(settings)
    } else {
        crate::clicks::list(settings)
    };
    crate::clicks::one_click_files(modes)
}

pub(crate) fn preview_target(entry: Option<FileEntry>) -> Option<FileEntry> {
    entry.filter(entry_supports_quick_preview)
}

/// Whether `entry` supports Quick Preview (`entry_supports_quick_preview` in
/// `src/ui/preview.rs`).
pub(crate) fn entry_supports_quick_preview(entry: &FileEntry) -> bool {
    if !matches!(entry.kind, EntryKind::File | EntryKind::FileSymbolicLink) {
        return false;
    }
    if strata_fs::services::is_model(entry)
        || strata_fs::services::route_preview::RouteFormat::for_name(&entry.native_name).is_some()
        || strata_preview::sandbox::CoverFormat::for_name(&entry.native_name).is_some()
    {
        return entry.location.native_path().is_some();
    }
    let native_path = entry.location.native_path();
    let (content_type, uncertain) =
        strata_fs::services::guess_content_type(Path::new(&entry.native_name));
    let content = strata_fs::services::content_family(&content_type);
    if strata_fs::services::table::is_workbook(&content_type, &entry.native_name)
        || strata_preview::sandbox::DocumentFormat::for_file(&content_type, &entry.native_name)
            .is_some()
    {
        return native_path.is_some();
    }
    if native_path.is_none() && remote_unsupported(&content, &entry.native_name) {
        return false;
    }
    if native_path.is_some()
        && strata_fs::services::archive_preview_format(&entry.native_name).is_some()
    {
        return true;
    }
    !matches!(content, PreviewContent::Unsupported)
        || uncertain
        || gio::content_type_is_a(&content_type, "text/plain")
        || strata_fs::services::has_plain_text_extension(&entry.native_name)
        || strata_fs::services::is_extensionless_dotfile(&entry.native_name)
}

fn remote_unsupported(content: &PreviewContent, native_name: &std::ffi::OsStr) -> bool {
    matches!(content, PreviewContent::Pdf { .. })
        || (matches!(content, PreviewContent::Media)
            && !strata_fs::services::supports_remote_video(native_name))
}

fn ensure_operations_watch(app: &mut App) {
    if app.preview.operations.is_some() {
        return;
    }
    let later = app.preview.loader.later.clone();
    let subscription = file_activity::observe(move |notice| match notice {
        Notice::Releasing(roots) => {
            let roots = roots.to_vec();
            later.then(move |app: &mut App, tx: &mut Tx| release_for_operation(app, &roots, tx));
        }
        Notice::Lifted(_) => {
            later.then(resume_after_operation);
        }
        Notice::Ended(_) => {}
    });
    app.preview.operations = Some(subscription);
}

fn release_for_operation(app: &mut App, roots: &[std::path::PathBuf], tx: &mut Tx) -> bool {
    let changing = app.preview.current.as_ref().is_some_and(|entry| {
        entry
            .local_thumbnail_path()
            .is_some_and(|path| roots.iter().any(|root| path.starts_with(root)))
    });
    if !changing {
        return false;
    }
    let depth = app.preview.current_depth;
    session::clear_target(app, tx);
    app.preview.released_from = Some(depth);
    true
}

fn resume_after_operation(app: &mut App, tx: &mut Tx) -> bool {
    let Some(depth) = app.preview.released_from else {
        return false;
    };
    let depth = depth.or_else(|| app.browser.active_depth());
    let entry = depth
        .and_then(|d| app.browser.cursor_entry(d))
        .and_then(|entry| preview_target(Some(entry)));
    if entry
        .as_ref()
        .and_then(FileEntry::local_thumbnail_path)
        .is_some_and(file_activity::is_changing)
    {
        return false;
    }
    app.preview.released_from = None;
    if let Some(entry) = entry.filter(|_| app.preview.enabled) {
        session::show_after_focus_change(app, entry, depth, tx);
        return true;
    }
    false
}
