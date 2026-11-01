// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use crate::services::{JobId, JobService, JobSnapshot, JobStatus, ListenerGuard};

use super::{actions::action_icon, modal::close_on_escape};

#[cfg(test)]
mod tests;

type RefreshCallback = Rc<dyn Fn()>;
type RefreshHolder = Rc<RefCell<Option<RefreshCallback>>>;
type ObserverHolder = Rc<RefCell<Option<ListenerGuard<RefreshCallback>>>>;
type StatusLabels = Rc<RefCell<Vec<(JobId, glib::WeakRef<gtk::Label>)>>>;

const PUMP_INTERVAL: Duration = Duration::from_millis(120);
const MAX_ROWS: usize = 12;

thread_local! {
    // Jobs must survive navigation and individual window closure.
    static SHARED_JOBS: RefCell<Option<Rc<JobService>>> = const { RefCell::new(None) };
    static PUMP_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn shared() -> Rc<JobService> {
    SHARED_JOBS.with(|shared| {
        let mut shared = shared.borrow_mut();
        if let Some(service) = shared.as_ref() {
            return service.clone();
        }
        let service = JobService::new(crate::adapters::LocalActionRunner::new());
        shared.replace(service.clone());
        service
    })
}

pub(crate) fn present_for(anchor: &impl IsA<gtk::Widget>, id: JobId) {
    let _ = anchor.activate_action("jobs.show", Some(&id.0.to_variant()));
}

// Install from window composition: GLib binds the source to its creating thread.
fn install_pump(service: &Rc<JobService>) {
    if PUMP_INSTALLED.replace(true) {
        return;
    }
    let pumped = service.clone();
    glib::timeout_add_local(PUMP_INTERVAL, move || {
        pumped.pump();
        glib::ControlFlow::Continue
    });
}

#[derive(Clone)]
struct DashboardState {
    service: Rc<JobService>,
    expanded: Rc<RefCell<HashSet<JobId>>>,
    dirty: Rc<RefCell<bool>>,
    featured: Rc<Cell<Option<JobId>>>,
    status_labels: StatusLabels,
    rows: Rc<RefCell<HashMap<JobId, RowWidgets>>>,
}

impl DashboardState {
    fn new(service: Rc<JobService>) -> Self {
        Self {
            service,
            expanded: Rc::new(RefCell::new(HashSet::new())),
            dirty: Rc::new(RefCell::new(true)),
            featured: Rc::new(Cell::new(None)),
            status_labels: Rc::new(RefCell::new(Vec::new())),
            rows: Rc::new(RefCell::new(HashMap::new())),
        }
    }
}

struct RowWidgets {
    root: glib::WeakRef<gtk::Box>,
    status: JobStatus,
    expanded: bool,
    status_label: glib::WeakRef<gtk::Label>,
    meta: glib::WeakRef<gtk::Label>,
    progress: Option<glib::WeakRef<gtk::ProgressBar>>,
    log: glib::WeakRef<gtk::Label>,
    created: glib::WeakRef<gtk::Label>,
}

impl RowWidgets {
    fn update(&self, snapshot: &JobSnapshot) {
        if let Some(label) = self.status_label.upgrade() {
            label.set_text(&status_label(snapshot));
        }
        if let Some(label) = self.meta.upgrade() {
            label.set_text(&meta_label(snapshot));
        }
        if let Some(progress) = self.progress.as_ref().and_then(glib::WeakRef::upgrade) {
            show_progress(&progress, snapshot);
        }
        if let Some(label) = self.log.upgrade() {
            show_log(&label, snapshot);
        }
        if let Some(label) = self.created.upgrade() {
            label.set_visible(!snapshot.created.is_empty());
            label.set_text(&rust_i18n::t!(
                "Reported %{value1} created.",
                value1 = item_count(snapshot.created.len())
            ));
        }
    }
}

fn show_progress(progress: &gtk::ProgressBar, snapshot: &JobSnapshot) {
    match snapshot.progress.fraction(snapshot.mode) {
        Some(fraction) => progress.set_fraction(fraction),
        None => progress.pulse(),
    }
}

fn show_log(label: &gtk::Label, snapshot: &JobSnapshot) {
    label.set_text(&log_text(snapshot));
    crate::ui::accessibility::set_description(
        label,
        snapshot
            .log_truncated
            .then(|| crate::i18n::tr("Older output was discarded to bound memory"))
            .as_deref(),
    );
}

fn log_text(snapshot: &JobSnapshot) -> String {
    let log = details_text(snapshot);
    if log.is_empty() {
        crate::i18n::tr("No output yet.")
    } else {
        log
    }
}

pub(crate) struct JobsIndicator {
    root: gtk::MenuButton,
    label: gtk::Label,
    clear: gtk::Button,
    scroll: gtk::ScrolledWindow,
    state: DashboardState,
    refresh_holder: RefreshHolder,
}

/// The dashboard's title row with its summaries and actions.
struct DashboardHeader {
    bar: gtk::Box,
    running: gtk::Label,
    finished: gtk::Label,
    clear: gtk::Button,
    minimize: gtk::Button,
}

impl DashboardHeader {
    fn build() -> Self {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        bar.add_css_class("jobs-header");
        bar.append(&icon_bezel(crate::assets::icons::LIST_CHECKS));
        let title = gtk::Label::builder()
            .label(crate::i18n::tr("Jobs"))
            .xalign(0.0)
            .build();
        title.add_css_class("jobs-title");
        let minimize = job_button("Minimize", crate::assets::icons::MINUS);
        bar.append(&title);
        let running = gtk::Label::new(None);
        running.add_css_class("jobs-running-summary");
        bar.append(&running);
        let finished = gtk::Label::builder().xalign(0.0).build();
        finished.add_css_class("jobs-finished-summary");
        bar.append(&finished);
        let header_space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header_space.set_hexpand(true);
        bar.append(&header_space);
        let clear = clear_button();
        bar.append(&clear);
        bar.append(&minimize);
        Self {
            bar,
            running,
            finished,
            clear,
            minimize,
        }
    }
}

impl JobsIndicator {
    pub(crate) fn new() -> Self {
        let service = shared();
        install_pump(&service);
        Self::with_service(service)
    }

    fn with_service(service: Rc<JobService>) -> Self {
        let state = DashboardState::new(service);
        let (root, label) = indicator_button();
        let popover = jobs_popover();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.set_overflow(gtk::Overflow::Hidden);
        let header = DashboardHeader::build();
        body.append(&header.bar);
        let (list, scroll) = jobs_list();
        body.append(&scroll);
        popover.set_child(Some(&body));
        root.set_popover(Some(&popover));

        let refresh_holder: RefreshHolder = Rc::new(RefCell::new(None));
        let refresh = dashboard_refresh(&refresh_holder, &list, &header, state.clone());
        refresh_holder.borrow_mut().replace(refresh.clone());
        follow_status_labels(&root, &state);
        connect_header_actions(&popover, &header, &state, &refresh);
        connect_popover(&popover, &body, &state, &refresh);

        let indicator = Self {
            root,
            label,
            clear: header.clear,
            scroll,
            state,
            refresh_holder,
        };
        indicator.subscribe();
        indicator.refresh_indicator();
        indicator
    }

    pub(crate) fn widget(&self) -> &gtk::MenuButton {
        &self.root
    }

    pub(crate) fn bind_window(&self, window: &impl IsA<gtk::Widget>) {
        let action = gio::SimpleAction::new("show", Some(&u64::static_variant_type()));
        let root = self.root.downgrade();
        let scroll = self.scroll.downgrade();
        let featured = self.state.featured.clone();
        let dirty = self.state.dirty.clone();
        let holder = Rc::downgrade(&self.refresh_holder);
        action.connect_activate(move |_, parameter| {
            let Some(id) = parameter.and_then(|value| value.get::<u64>()) else {
                return;
            };
            featured.set(Some(JobId(id)));
            dirty.replace(true);
            let root = root.clone();
            let scroll = scroll.clone();
            let holder = holder.clone();
            // Let the launching menu release its grab and the footer become visible.
            glib::idle_add_local_once(move || {
                let (Some(root), Some(scroll), Some(holder)) =
                    (root.upgrade(), scroll.upgrade(), holder.upgrade())
                else {
                    return;
                };
                if !root.is_mapped() {
                    return;
                }
                if let Some(refresh) = holder.borrow().clone() {
                    refresh();
                }
                scroll.vadjustment().set_value(scroll.vadjustment().lower());
                root.popup();
            });
        });
        let group = gio::SimpleActionGroup::new();
        group.add_action(&action);
        window.insert_action_group("jobs", Some(&group));
        if let Some(window) = window.as_ref().downcast_ref::<gtk::Window>() {
            let service = self.state.service.clone();
            crate::ui::close_guard::install(window, move |closing_application| {
                (closing_application && service.running_count() + service.queued_count() > 0).then(|| {
                    crate::ui::close_guard::CloseBlocker {
                        title: crate::i18n::tr("Background jobs are still active"),
                        detail: crate::i18n::tr("Wait for Jobs to finish, or cancel them in the Jobs dashboard before closing the last window. Cancellation does not undo file changes."),
                    }
                })
            });
        }
    }

    fn subscribe(&self) {
        let weak_label = self.label.downgrade();
        let weak_root = self.root.downgrade();
        let weak_clear = self.clear.downgrade();
        let service = self.state.service.clone();
        let dirty = self.state.dirty.clone();
        let weak_refresh = Rc::downgrade(&self.refresh_holder);
        let callback: Rc<dyn Fn()> = Rc::new(move || {
            dirty.replace(true);
            if let Some(label) = weak_label.upgrade() {
                show_indicator_label(&label, &weak_root, &service);
            }
            if let Some(clear) = weak_clear.upgrade() {
                clear.set_visible(service.finished_count() > 0);
            }
            if let Some(root) = weak_root.upgrade() {
                refresh_open_dashboard(&root, &weak_refresh);
            }
        });
        let guard: ObserverHolder = Rc::new(RefCell::new(None));
        guard
            .borrow_mut()
            .replace(self.state.service.observe(callback));
        // Window composition retains only the widget, not this builder.
        let refresh_holder = self.refresh_holder.clone();
        self.root.connect_destroy(move |_| {
            guard.borrow_mut().take();
            refresh_holder.borrow_mut().take();
        });
    }

    fn refresh_indicator(&self) {
        let text = indicator_label(&self.state.service);
        self.label.set_text(&text);
        self.root.set_visible(!text.is_empty());
        self.clear
            .set_visible(self.state.service.finished_count() > 0);
    }
}

fn show_indicator_label(
    label: &gtk::Label,
    root: &glib::WeakRef<gtk::MenuButton>,
    service: &JobService,
) {
    let text = indicator_label(service);
    let visible = !text.is_empty();
    label.set_text(&text);
    if let Some(root) = root.upgrade() {
        root.set_visible(visible);
    }
}

fn refresh_open_dashboard(
    root: &gtk::MenuButton,
    holder: &std::rc::Weak<RefCell<Option<RefreshCallback>>>,
) {
    if !root.popover().is_some_and(|popover| popover.is_visible()) {
        return;
    }
    if let Some(holder) = holder.upgrade()
        && let Some(refresh) = holder.borrow().clone()
    {
        refresh();
    }
}

fn indicator_button() -> (gtk::MenuButton, gtk::Label) {
    let root = gtk::MenuButton::new();
    root.set_direction(gtk::ArrowType::Up);
    root.add_css_class("shortcut-footer-button");
    root.add_css_class("jobs-indicator");
    crate::ui::accessibility::set_description(
        &root,
        Some(&crate::i18n::tr("Show background jobs")),
    );
    let label = gtk::Label::new(None);
    label.add_css_class("jobs-indicator-label");
    root.set_child(Some(&label));
    root.set_visible(false);
    (root, label)
}

fn jobs_popover() -> gtk::Popover {
    let popover = gtk::Popover::builder()
        .position(gtk::PositionType::Top)
        .halign(gtk::Align::End)
        .has_arrow(false)
        .build();
    popover.add_css_class("jobs-popover");
    super::preferences::PreferenceManager::shared().bind_interface_scale(
        &popover,
        |widget, scale| {
            if let Some(popover) = widget.downcast_ref::<gtk::Popover>() {
                popover.set_offset(0, -(12.0 * scale).round() as i32);
            }
        },
    );
    popover
}

fn clear_button() -> gtk::Button {
    let clear = job_button("Clear finished", crate::assets::icons::TRASH);
    let clear_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    clear_content.append(&crate::assets::primary_icon(
        crate::assets::icons::TRASH,
        14,
    ));
    clear_content.append(&gtk::Label::new(Some(&crate::i18n::tr("Clear finished"))));
    clear.set_child(Some(&clear_content));
    clear.add_css_class("jobs-clear");
    clear.set_visible(false);
    clear
}

fn jobs_list() -> (gtk::Box, gtk::ScrolledWindow) {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.add_css_class("jobs-list");
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(420)
        .width_request(500)
        .build();
    (list, scroll)
}

/// Rows retain this callback; widget references must stay weak to avoid cycles.
fn dashboard_refresh(
    holder: &RefreshHolder,
    list: &gtk::Box,
    header: &DashboardHeader,
    state: DashboardState,
) -> RefreshCallback {
    let weak_holder = Rc::downgrade(holder);
    let weak_list = list.downgrade();
    let weak_clear = header.clear.downgrade();
    let weak_running = header.running.downgrade();
    let weak_finished = header.finished.downgrade();
    Rc::new(move || {
        let Some(holder) = weak_holder.upgrade() else {
            return;
        };
        let Some(callback) = holder.borrow().clone() else {
            return;
        };
        let (Some(list), Some(clear)) = (weak_list.upgrade(), weak_clear.upgrade()) else {
            return;
        };
        if let (Some(running), Some(finished)) = (weak_running.upgrade(), weak_finished.upgrade()) {
            show_activity_summary(&running, &finished, &state.service);
        }
        render_rows(&list, &clear, &state, &callback);
    })
}

fn show_activity_summary(running: &gtk::Label, finished: &gtk::Label, service: &JobService) {
    let active = service.running_count();
    let queued = service.queued_count();
    let history = service.finished_count();
    running.set_text(&dashboard_activity_label(active, queued));
    running.set_visible(active + queued > 0);
    finished.set_text(&format!(
        "{}{}",
        if active + queued > 0 { "· " } else { "" },
        rust_i18n::t!("%{count} finished", count = history)
    ));
    finished.set_visible(history > 0);
}

/// Keeps the elapsed time of active rows current while the dashboard is open.
fn follow_status_labels(root: &gtk::MenuButton, state: &DashboardState) {
    let root_weak = root.downgrade();
    let service_weak = Rc::downgrade(&state.service);
    let status_labels = state.status_labels.clone();
    glib::timeout_add_local(Duration::from_secs(1), move || {
        let (Some(root), Some(service)) = (root_weak.upgrade(), service_weak.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        if root.popover().is_some_and(|popover| popover.is_visible()) {
            refresh_status_labels(&service, &status_labels);
        }
        glib::ControlFlow::Continue
    });
}

fn refresh_status_labels(service: &JobService, status_labels: &StatusLabels) {
    let snapshots = service.snapshot();
    for (id, label) in status_labels.borrow().iter() {
        if let (Some(label), Some(snapshot)) = (
            label.upgrade(),
            snapshots.iter().find(|snapshot| snapshot.id == *id),
        ) {
            label.set_text(&status_label(snapshot));
        }
    }
}

fn connect_header_actions(
    popover: &gtk::Popover,
    header: &DashboardHeader,
    state: &DashboardState,
    refresh: &RefreshCallback,
) {
    let weak_popover = popover.downgrade();
    header.minimize.connect_clicked(move |_| {
        if let Some(popover) = weak_popover.upgrade() {
            popover.popdown();
        }
    });
    let clear_service = state.service.clone();
    let clear_refresh = refresh.clone();
    header.clear.connect_clicked(move |_| {
        if clear_service.clear_finished() {
            clear_refresh();
        }
    });
}

fn connect_popover(
    popover: &gtk::Popover,
    body: &gtk::Box,
    state: &DashboardState,
    refresh: &RefreshCallback,
) {
    let show_refresh = refresh.clone();
    let show_state = state.clone();
    popover.connect_show(move |_| {
        show_state.dirty.replace(true);
        show_refresh();
    });
    let hide_dirty = state.dirty.clone();
    popover.connect_closed(move |_| {
        hide_dirty.replace(true);
    });

    let weak_popover = popover.downgrade();
    close_on_escape(body, move || {
        if let Some(popover) = weak_popover.upgrade() {
            popover.popdown();
        }
    });
}

fn render_rows(
    list: &gtk::Box,
    clear: &gtk::Button,
    state: &DashboardState,
    refresh: &RefreshCallback,
) {
    if !state.dirty.replace(false) && list.first_child().is_some() {
        return;
    }
    state.status_labels.borrow_mut().clear();
    let snapshots = dashboard_snapshots(state);
    let widgets = render_shown_rows(&snapshots, state, refresh);
    remove_stale_rows(list, &widgets);
    place_rows(list, &widgets);
    clear.set_visible(state.service.finished_count() > 0);
    append_list_note(list, snapshots.len());
}

fn render_shown_rows(
    snapshots: &[JobSnapshot],
    state: &DashboardState,
    refresh: &RefreshCallback,
) -> Vec<gtk::Box> {
    let shown: Vec<_> = snapshots.iter().take(MAX_ROWS).collect();
    let mut rows = state.rows.borrow_mut();
    rows.retain(|id, _| shown.iter().any(|snapshot| snapshot.id == *id));
    shown
        .iter()
        .map(|snapshot| render_row(&mut rows, snapshot, state, refresh))
        .collect()
}

/// Reuses a row whose status and expansion are unchanged, else builds a new one.
fn render_row(
    rows: &mut HashMap<JobId, RowWidgets>,
    snapshot: &JobSnapshot,
    state: &DashboardState,
    refresh: &RefreshCallback,
) -> gtk::Box {
    let expanded = state.expanded.borrow().contains(&snapshot.id);
    let existing = rows
        .get(&snapshot.id)
        .filter(|row| row.status == snapshot.status && row.expanded == expanded)
        .and_then(|row| row.root.upgrade());
    let widget = existing.unwrap_or_else(|| {
        let (widget, binding) = job_row(snapshot, state, refresh);
        rows.insert(snapshot.id, binding);
        widget
    });
    let binding = rows.get(&snapshot.id).expect("rendered job row");
    binding.update(snapshot);
    if snapshot.is_active() {
        state
            .status_labels
            .borrow_mut()
            .push((snapshot.id, binding.status_label.clone()));
    }
    widget
}

fn remove_stale_rows(list: &gtk::Box, widgets: &[gtk::Box]) {
    let mut child = list.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if !widgets
            .iter()
            .any(|row| row.upcast_ref::<gtk::Widget>() == &widget)
        {
            list.remove(&widget);
        }
    }
}

fn place_rows(list: &gtk::Box, widgets: &[gtk::Box]) {
    let mut previous: Option<&gtk::Box> = None;
    for widget in widgets {
        if widget.parent().is_none() {
            list.append(widget);
        }
        list.reorder_child_after(widget, previous);
        previous = Some(widget);
    }
}

fn append_list_note(list: &gtk::Box, job_count: usize) {
    if job_count == 0 {
        append_note(list, &crate::i18n::tr("No background jobs."));
        return;
    }
    if job_count > MAX_ROWS {
        append_note(
            list,
            &format!("{} older jobs are hidden", job_count - MAX_ROWS),
        );
    }
}

fn append_note(list: &gtk::Box, text: &str) {
    let note = gtk::Label::new(Some(text));
    note.add_css_class("jobs-empty");
    note.set_xalign(0.0);
    list.append(&note);
}

fn dashboard_snapshots(state: &DashboardState) -> Vec<JobSnapshot> {
    let mut snapshots = state.service.snapshot();
    if let Some(id) = state.featured.get() {
        if let Some(index) = snapshots.iter().position(|snapshot| snapshot.id == id) {
            let featured = snapshots.remove(index);
            snapshots.insert(0, featured);
        } else {
            state.featured.set(None);
        }
    }
    snapshots
}

fn job_row(
    snapshot: &JobSnapshot,
    state: &DashboardState,
    refresh: &RefreshCallback,
) -> (gtk::Box, RowWidgets) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("job-row");
    row.add_css_class(match snapshot.status {
        JobStatus::Succeeded => "job-succeeded",
        JobStatus::Failed => "job-failed",
        JobStatus::Cancelled => "job-cancelled",
        _ => "job-active",
    });

    let bezel = icon_bezel(job_icon(snapshot));
    bezel.set_valign(gtk::Align::Start);
    row.append(&bezel);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 7);
    content.set_hexpand(true);
    row.append(&content);
    let status = job_row_header(&content, snapshot, state, refresh);

    let meta = gtk::Label::builder()
        .label(meta_label(snapshot))
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(52)
        .build();
    meta.add_css_class("job-meta");
    content.append(&meta);

    let progress = snapshot
        .is_active()
        .then(|| job_progress(&content, snapshot).downgrade());
    let (log, created) = job_detail(&content, snapshot, state);
    let widgets = RowWidgets {
        root: row.downgrade(),
        status: snapshot.status,
        expanded: state.expanded.borrow().contains(&snapshot.id),
        status_label: status.downgrade(),
        meta: meta.downgrade(),
        progress,
        log: log.downgrade(),
        created: created.downgrade(),
    };
    (row, widgets)
}

/// Appends the name, status and controls row, returning the status label.
fn job_row_header(
    content: &gtk::Box,
    snapshot: &JobSnapshot,
    state: &DashboardState,
    refresh: &RefreshCallback,
) -> gtk::Label {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_valign(gtk::Align::Center);
    let name = gtk::Label::builder()
        .label(snapshot.action_name.clone())
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(24)
        .valign(gtk::Align::Center)
        .build();
    name.add_css_class("job-name");
    header.append(&name);
    let status = gtk::Label::new(Some(&status_label(snapshot)));
    status.add_css_class("job-status");
    status.set_valign(gtk::Align::Center);
    header.append(&status);
    if let Some(icon) = status_icon(snapshot.status) {
        let icon = crate::assets::primary_icon(icon, 14);
        icon.set_valign(gtk::Align::Center);
        header.append(&icon);
    }
    let space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    space.set_hexpand(true);
    header.append(&space);
    for button in job_controls(snapshot, state, refresh) {
        header.append(&button);
    }
    content.append(&header);
    status
}

fn job_progress(content: &gtk::Box, snapshot: &JobSnapshot) -> gtk::ProgressBar {
    let progress = gtk::ProgressBar::new();
    progress.add_css_class("job-progress");
    progress.set_show_text(false);
    show_progress(&progress, snapshot);
    content.append(&progress);
    progress
}

/// Appends the expandable log, returning its label and the created-items label.
fn job_detail(
    content: &gtk::Box,
    snapshot: &JobSnapshot,
    state: &DashboardState,
) -> (gtk::Label, gtk::Label) {
    let detail = gtk::Box::new(gtk::Orientation::Vertical, 4);
    detail.add_css_class("job-detail");
    let log_label = job_log_label(snapshot);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&log_label)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(160)
        .build();
    follow_log_output(&scroll);
    detail.append(&scroll);
    let created = gtk::Label::new(None);
    created.set_xalign(0.0);
    created.add_css_class("settings-option-description");
    detail.append(&created);
    let revealer = gtk::Revealer::builder()
        .child(&detail)
        .transition_duration(0)
        .reveal_child(state.expanded.borrow().contains(&snapshot.id))
        .build();
    content.append(&revealer);
    (log_label, created)
}

fn job_log_label(snapshot: &JobSnapshot) -> gtk::Label {
    let log_label = gtk::Label::builder()
        .label(log_text(snapshot))
        .xalign(0.0)
        .yalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .max_width_chars(60)
        .build();
    log_label.add_css_class("job-log");
    if snapshot.log_truncated {
        crate::ui::accessibility::set_description(
            &log_label,
            Some(&crate::i18n::tr(
                "Older output was discarded to bound memory",
            )),
        );
    }
    log_label
}

fn follow_log_output(scroll: &gtk::ScrolledWindow) {
    let adjustment = scroll.vadjustment();
    let following = Rc::new(Cell::new(true));
    let track_position = following.clone();
    adjustment.connect_value_changed(move |adjustment| {
        track_position.set(at_log_bottom(adjustment));
    });
    adjustment.connect_changed(move |adjustment| {
        if following.get() {
            adjustment
                .set_value((adjustment.upper() - adjustment.page_size()).max(adjustment.lower()));
        }
    });
}

fn job_controls(
    snapshot: &JobSnapshot,
    state: &DashboardState,
    refresh: &RefreshCallback,
) -> Vec<gtk::Button> {
    let mut buttons = Vec::new();
    if matches!(snapshot.status, JobStatus::Running | JobStatus::Queued) {
        buttons.push(cancel_button(snapshot, state, refresh));
    }
    if snapshot.status != JobStatus::Queued {
        buttons.push(details_button(snapshot, state, refresh));
    }
    if snapshot.status.is_finished() {
        buttons.push(dismiss_button(snapshot, state, refresh));
    }
    buttons
}

fn cancel_button(
    snapshot: &JobSnapshot,
    state: &DashboardState,
    refresh: &RefreshCallback,
) -> gtk::Button {
    let label = if snapshot.status == JobStatus::Queued {
        "Remove"
    } else {
        "Cancel"
    };
    let button = job_button(label, crate::assets::icons::X);
    let service = state.service.clone();
    let id = snapshot.id;
    let refresh = refresh.clone();
    button.connect_clicked(move |_| {
        service.cancel(id);
        refresh();
    });
    button
}

fn details_button(
    snapshot: &JobSnapshot,
    state: &DashboardState,
    refresh: &RefreshCallback,
) -> gtk::Button {
    let expanded = state.expanded.borrow().contains(&snapshot.id);
    let details = job_button(
        if expanded { "Hide" } else { "Details" },
        if expanded {
            crate::assets::icons::EYE_OFF
        } else {
            crate::assets::icons::EYE
        },
    );
    let id = snapshot.id;
    let expanded_state = state.expanded.clone();
    let details_refresh = refresh.clone();
    let dirty = state.dirty.clone();
    details.connect_clicked(move |_| {
        let mut expanded = expanded_state.borrow_mut();
        if !expanded.remove(&id) {
            expanded.insert(id);
        }
        drop(expanded);
        dirty.replace(true);
        details_refresh();
    });
    details
}

fn dismiss_button(
    snapshot: &JobSnapshot,
    state: &DashboardState,
    refresh: &RefreshCallback,
) -> gtk::Button {
    let dismiss = job_button("Dismiss", crate::assets::icons::X);
    let service = state.service.clone();
    let id = snapshot.id;
    let dismiss_refresh = refresh.clone();
    dismiss.connect_clicked(move |_| {
        service.dismiss(id);
        dismiss_refresh();
    });
    dismiss
}

fn job_button(label: &str, icon: &str) -> gtk::Button {
    let label = &crate::i18n::tr(label);
    let image = crate::assets::primary_icon(icon, crate::assets::CHROME_ICON_PX);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    let button = gtk::Button::builder()
        .child(&image)
        .tooltip_text(label)
        .build();
    button.add_css_class("job-action");
    button.set_valign(gtk::Align::Center);
    crate::ui::accessibility::set_label(&button, label);
    button
}

pub(crate) fn indicator_label(service: &JobService) -> String {
    let running = service.running_count();
    let queued = service.queued_count();
    let mut parts = Vec::new();
    if running > 0 {
        parts.push(crate::i18n::count("jobs_running", running));
    }
    if queued > 0 {
        parts.push(rust_i18n::t!("%{count} queued", count = queued).into_owned());
    }
    if !parts.is_empty() {
        return parts.join(" · ");
    }
    let finished = service.finished_count();
    if finished == 0 {
        return String::new();
    }
    let jobs = crate::i18n::count("jobs_finished", finished);
    if service.has_failures() {
        rust_i18n::t!("%{jobs} · failures", jobs = jobs).into_owned()
    } else {
        jobs
    }
}

fn status_label(snapshot: &JobSnapshot) -> String {
    let elapsed = format_elapsed(snapshot.elapsed);
    match snapshot.status {
        JobStatus::Succeeded => rust_i18n::t!("Done in %{elapsed}", elapsed = elapsed).into_owned(),
        JobStatus::Failed => {
            rust_i18n::t!("Failed after %{elapsed}", elapsed = elapsed).into_owned()
        }
        JobStatus::Cancelled => {
            rust_i18n::t!("Cancelled after %{elapsed}", elapsed = elapsed).into_owned()
        }
        JobStatus::Cancelling => crate::i18n::tr("Cancelling…"),
        JobStatus::Queued => crate::i18n::tr("Queued"),
        JobStatus::Running => {
            rust_i18n::t!("Running for %{elapsed}", elapsed = elapsed).into_owned()
        }
    }
}

fn status_icon(status: JobStatus) -> Option<&'static str> {
    match status {
        JobStatus::Succeeded => Some(crate::assets::icons::CHECK),
        JobStatus::Failed | JobStatus::Cancelled => Some(crate::assets::icons::X),
        _ => None,
    }
}

fn dashboard_activity_label(running: usize, queued: usize) -> String {
    match (running, queued) {
        (0, queued) => rust_i18n::t!("%{count} queued", count = queued).into_owned(),
        (running, 0) => rust_i18n::t!("%{count} running", count = running).into_owned(),
        (running, queued) => format!(
            "{} · {}",
            rust_i18n::t!("%{count} running", count = running),
            rust_i18n::t!("%{count} queued", count = queued)
        ),
    }
}

fn icon_bezel(icon: &str) -> gtk::Box {
    let bezel = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bezel.add_css_class("job-icon-bezel");
    let image = crate::assets::primary_icon(icon, 16);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    bezel.append(&image);
    bezel
}

fn details_text(snapshot: &JobSnapshot) -> String {
    let log = strip_terminal_sequences(snapshot.log.trim_end());
    let message = snapshot
        .message
        .as_deref()
        .filter(|_| snapshot.status != JobStatus::Succeeded);
    if let Some(message) = message
        && !log.contains(message)
    {
        return format!("{message}\n{log}").trim_end().to_owned();
    }
    log
}

fn at_log_bottom(adjustment: &gtk::Adjustment) -> bool {
    adjustment.value() >= adjustment.upper() - adjustment.page_size() - 2.0
}

fn strip_terminal_sequences(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if character == '\x1b' {
            skip_escape_sequence(&mut chars);
        } else if keeps_character(character) {
            clean.push(character);
        }
    }
    clean
}

fn keeps_character(character: char) -> bool {
    !character.is_control() || matches!(character, '\n' | '\t')
}

fn skip_escape_sequence(chars: &mut std::str::Chars<'_>) {
    match chars.next() {
        Some('[') => skip_control_sequence(chars),
        Some(']') => skip_operating_system_command(chars),
        _ => {}
    }
}

fn skip_control_sequence(chars: &mut std::str::Chars<'_>) {
    for character in chars.by_ref() {
        if ('@'..='~').contains(&character) {
            break;
        }
    }
}

/// Skips to the BEL or ESC \ that ends the command.
fn skip_operating_system_command(chars: &mut std::str::Chars<'_>) {
    while let Some(character) = chars.next() {
        if character == '\x07' {
            break;
        }
        if character == '\x1b' && chars.next() == Some('\\') {
            break;
        }
    }
}

fn meta_label(snapshot: &JobSnapshot) -> String {
    let mut parts = Vec::new();
    if let Some(message) = &snapshot.progress.message {
        parts.push(message.clone());
    }
    if snapshot.mode == crate::model::ExecutionMode::PerItem && snapshot.progress.total_items > 0 {
        parts.push(format!(
            "{}/{}",
            crate::i18n::integer(snapshot.progress.completed_items as u64),
            crate::i18n::integer(snapshot.progress.total_items as u64)
        ));
        if snapshot.progress.failed_items > 0 {
            parts.push(
                rust_i18n::t!("%{count} failed", count = snapshot.progress.failed_items)
                    .into_owned(),
            );
        }
    } else if let Some(script) = &snapshot.progress.script
        && let Some(total) = script.total.filter(|total| *total > 0)
    {
        parts.push(format!(
            "{}/{}",
            crate::i18n::integer(script.completed as u64),
            crate::i18n::integer(total as u64)
        ));
    }
    parts.push(compact_home(&snapshot.parent));
    parts.join(" · ")
}

fn compact_home(path: &std::path::Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => crate::i18n::tr("Home"),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn format_elapsed(elapsed: Duration) -> String {
    crate::i18n::duration(elapsed.as_secs())
}

fn item_count(count: usize) -> String {
    crate::i18n::count("items", count)
}

pub(crate) fn job_icon(snapshot: &JobSnapshot) -> &'static str {
    action_icon(snapshot.icon.as_deref())
}
