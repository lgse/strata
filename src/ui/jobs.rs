// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use crate::services::{JobId, JobService, JobSnapshot, JobStatus, ListenerGuard};

use super::actions::action_icon;

type RefreshCallback = Rc<dyn Fn()>;
type RefreshHolder = Rc<RefCell<Option<RefreshCallback>>>;
type ObserverHolder = Rc<RefCell<Option<ListenerGuard<RefreshCallback>>>>;

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

struct DashboardState {
    service: Rc<JobService>,
    expanded: Rc<RefCell<HashSet<JobId>>>,
    dirty: Rc<RefCell<bool>>,
    featured: Rc<Cell<Option<JobId>>>,
}

pub(crate) struct JobsIndicator {
    root: gtk::MenuButton,
    label: gtk::Label,
    clear: gtk::Button,
    scroll: gtk::ScrolledWindow,
    state: DashboardState,
    refresh_holder: RefreshHolder,
}

impl JobsIndicator {
    pub(crate) fn new() -> Self {
        let service = shared();
        install_pump(&service);
        Self::with_service(service)
    }

    fn with_service(service: Rc<JobService>) -> Self {
        let state = DashboardState {
            service,
            expanded: Rc::new(RefCell::new(HashSet::new())),
            dirty: Rc::new(RefCell::new(true)),
            featured: Rc::new(Cell::new(None)),
        };

        let root = gtk::MenuButton::new();
        root.add_css_class("shortcut-footer-button");
        root.add_css_class("jobs-indicator");
        root.set_tooltip_text(Some("Show background jobs"));
        let label = gtk::Label::new(None);
        label.add_css_class("jobs-indicator-label");
        root.set_child(Some(&label));
        root.set_visible(false);

        let popover = gtk::Popover::builder()
            .position(gtk::PositionType::Top)
            .halign(gtk::Align::End)
            .has_arrow(false)
            .build();
        popover.add_css_class("shortcut-popover");
        popover.add_css_class("jobs-popover");
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let title = gtk::Label::builder()
            .label("Jobs")
            .xalign(0.0)
            .hexpand(true)
            .build();
        title.add_css_class("shortcut-reference-title");
        let minimize = job_button("Minimize", crate::assets::icons::MINUS);
        header.append(&title);
        header.append(&minimize);
        body.append(&header);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("jobs-list");
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height(420)
            .width_request(420)
            .build();
        scroll.add_css_class("fixed-scrollbar");
        body.append(&scroll);
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.set_halign(gtk::Align::End);
        let clear = job_button("Clear finished", crate::assets::icons::TRASH);
        clear.set_visible(false);
        footer.append(&clear);
        body.append(&footer);
        popover.set_child(Some(&body));
        root.set_popover(Some(&popover));

        // Rows retain this callback; widget references must stay weak to avoid cycles.
        let refresh_holder: RefreshHolder = Rc::new(RefCell::new(None));
        let weak_holder = Rc::downgrade(&refresh_holder);
        let weak_list = list.downgrade();
        let weak_clear = clear.downgrade();
        let refresh_state = DashboardState {
            service: state.service.clone(),
            expanded: state.expanded.clone(),
            dirty: state.dirty.clone(),
            featured: state.featured.clone(),
        };
        let refresh: RefreshCallback = Rc::new(move || {
            let Some(holder) = weak_holder.upgrade() else {
                return;
            };
            let Some(callback) = holder.borrow().clone() else {
                return;
            };
            let (Some(list), Some(clear)) = (weak_list.upgrade(), weak_clear.upgrade()) else {
                return;
            };
            render_rows(&list, &clear, &refresh_state, &callback);
        });
        refresh_holder.borrow_mut().replace(refresh.clone());

        let weak_popover = popover.downgrade();
        minimize.connect_clicked(move |_| {
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
        });
        let clear_service = state.service.clone();
        let clear_refresh = refresh.clone();
        clear.connect_clicked(move |_| {
            if clear_service.clear_finished() {
                clear_refresh();
            }
        });

        let show_refresh = refresh.clone();
        let show_state = DashboardState {
            service: state.service.clone(),
            expanded: state.expanded.clone(),
            dirty: state.dirty.clone(),
            featured: state.featured.clone(),
        };
        popover.connect_show(move |_| {
            show_state.dirty.replace(true);
            show_refresh();
        });
        let hide_dirty = state.dirty.clone();
        popover.connect_closed(move |_| {
            hide_dirty.replace(true);
        });

        let escape = gtk::EventControllerKey::new();
        let weak_popover = popover.downgrade();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(popover) = weak_popover.upgrade() {
                    popover.popdown();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        body.add_controller(escape);

        let indicator = Self {
            root,
            label,
            clear,
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
            window.connect_close_request(move |window| {
                let last_window = window.application()
                    .is_some_and(|application| application.windows().len() == 1);
                if last_window && service.running_count() + service.queued_count() > 0 {
                    crate::ui::modal::show_error_dialog(
                        window,
                        "Background jobs are still active",
                        "Wait for Jobs to finish, or cancel them in the Jobs dashboard before closing the last window. Cancellation does not undo file changes.",
                    );
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
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
                let text = indicator_label(&service);
                let visible = !text.is_empty();
                label.set_text(&text);
                if let Some(root) = weak_root.upgrade() {
                    root.set_visible(visible);
                }
            }
            if let Some(clear) = weak_clear.upgrade() {
                clear.set_visible(service.finished_count() > 0);
            }
            if let Some(root) = weak_root.upgrade()
                && root.popover().is_some_and(|popover| popover.is_visible())
                && let Some(holder) = weak_refresh.upgrade()
                && let Some(refresh) = holder.borrow().clone()
            {
                refresh();
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

fn render_rows(
    list: &gtk::Box,
    clear: &gtk::Button,
    state: &DashboardState,
    refresh: &RefreshCallback,
) {
    if !state.dirty.replace(false) && list.first_child().is_some() {
        return;
    }
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let snapshots = dashboard_snapshots(state);
    clear.set_visible(state.service.finished_count() > 0);
    if snapshots.is_empty() {
        let empty = gtk::Label::new(Some("No background jobs."));
        empty.add_css_class("settings-option-description");
        empty.set_xalign(0.0);
        list.append(&empty);
        return;
    }
    for snapshot in snapshots.iter().take(MAX_ROWS) {
        list.append(&job_row(snapshot, state, refresh));
    }
    if snapshots.len() > MAX_ROWS {
        let more = gtk::Label::new(Some(&format!(
            "{} older jobs are hidden",
            snapshots.len() - MAX_ROWS
        )));
        more.add_css_class("settings-option-description");
        more.set_xalign(0.0);
        list.append(&more);
    }
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

fn job_row(snapshot: &JobSnapshot, state: &DashboardState, refresh: &RefreshCallback) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
    row.add_css_class("job-row");
    row.add_css_class(match snapshot.status {
        JobStatus::Succeeded => "job-succeeded",
        JobStatus::Failed => "job-failed",
        JobStatus::Cancelled => "job-cancelled",
        _ => "job-active",
    });

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = crate::assets::primary_icon(job_icon(snapshot), 15);
    icon.add_css_class("job-icon");
    header.append(&icon);
    let name = gtk::Label::builder()
        .label(snapshot.action_name.clone())
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    name.add_css_class("job-name");
    header.append(&name);
    let status = gtk::Label::new(Some(&status_label(snapshot)));
    status.add_css_class("job-status");
    header.append(&status);
    for button in job_controls(snapshot, state, refresh) {
        header.append(&button);
    }
    row.append(&header);

    let meta = gtk::Label::builder()
        .label(meta_label(snapshot))
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
    meta.add_css_class("job-meta");
    row.append(&meta);

    if snapshot.is_active() {
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("job-progress");
        progress.set_show_text(false);
        match snapshot.progress.fraction(snapshot.mode) {
            Some(fraction) => progress.set_fraction(fraction),
            None => progress.pulse(),
        }
        row.append(&progress);
    }

    let detail = gtk::Box::new(gtk::Orientation::Vertical, 4);
    detail.add_css_class("job-detail");
    let log = snapshot.log.trim_end().to_owned();
    let log_label = gtk::Label::builder()
        .label(if log.is_empty() {
            "No output yet.".to_owned()
        } else {
            log
        })
        .xalign(0.0)
        .yalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .build();
    log_label.add_css_class("job-log");
    if snapshot.log_truncated {
        log_label.set_tooltip_text(Some("Older output was discarded to bound memory"));
    }
    let scroll = gtk::ScrolledWindow::builder()
        .child(&log_label)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(160)
        .build();
    detail.append(&scroll);
    if !snapshot.created.is_empty() {
        let created = gtk::Label::new(Some(&format!(
            "Reported {} created.",
            item_count(snapshot.created.len())
        )));
        created.set_xalign(0.0);
        created.add_css_class("settings-option-description");
        detail.append(&created);
    }
    let revealer = gtk::Revealer::builder()
        .child(&detail)
        .transition_duration(0)
        .reveal_child(state.expanded.borrow().contains(&snapshot.id))
        .build();
    row.append(&revealer);
    row
}

fn job_controls(
    snapshot: &JobSnapshot,
    state: &DashboardState,
    refresh: &RefreshCallback,
) -> Vec<gtk::Button> {
    let mut buttons = Vec::new();
    match snapshot.status {
        JobStatus::Running | JobStatus::Queued => {
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
            buttons.push(button);
        }
        JobStatus::Cancelling => {}
        _ => {
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
            buttons.push(details);

            let dismiss = job_button("Dismiss", crate::assets::icons::X);
            let service = state.service.clone();
            let id = snapshot.id;
            let dismiss_refresh = refresh.clone();
            dismiss.connect_clicked(move |_| {
                service.dismiss(id);
                dismiss_refresh();
            });
            buttons.push(dismiss);
        }
    }
    buttons
}

fn job_button(label: &str, icon: &str) -> gtk::Button {
    let image = crate::assets::primary_icon(icon, crate::assets::CHROME_ICON_PX);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    let button = gtk::Button::builder()
        .child(&image)
        .tooltip_text(label)
        .build();
    button.add_css_class("job-action");
    crate::ui::accessibility::set_label(&button, label);
    button
}

pub(crate) fn indicator_label(service: &JobService) -> String {
    let running = service.running_count();
    let queued = service.queued_count();
    let mut parts = Vec::new();
    if running > 0 {
        parts.push(format!(
            "{running} {}",
            plural(running, "job running", "jobs running")
        ));
    }
    if queued > 0 {
        parts.push(format!("{queued} queued"));
    }
    if !parts.is_empty() {
        return parts.join(" · ");
    }
    let finished = service.finished_count();
    if finished == 0 {
        return String::new();
    }
    let suffix = if service.has_failures() {
        "finished · failures"
    } else {
        "finished"
    };
    format!("{finished} {} {suffix}", plural(finished, "job", "jobs"))
}

fn status_label(snapshot: &JobSnapshot) -> String {
    let progress = &snapshot.progress;
    match snapshot.status {
        JobStatus::Succeeded => "Completed".to_owned(),
        JobStatus::Failed if progress.partial_success() => {
            format!("{} failed", item_count(progress.failed_items))
        }
        JobStatus::Failed => "Failed".to_owned(),
        JobStatus::Cancelled => "Cancelled".to_owned(),
        JobStatus::Cancelling => "Cancelling…".to_owned(),
        JobStatus::Queued => "Queued".to_owned(),
        JobStatus::Running => match snapshot.mode {
            crate::model::ExecutionMode::PerItem if progress.total_items > 0 => format!(
                "{} / {}",
                progress.completed_items.min(progress.total_items),
                progress.total_items
            ),
            _ => "Running".to_owned(),
        },
    }
}

fn meta_label(snapshot: &JobSnapshot) -> String {
    let mut parts = Vec::new();
    if let Some(label) = snapshot.progress.active_label() {
        parts.push(label);
    }
    parts.push(format!("{} elapsed", format_elapsed(snapshot.elapsed)));
    parts.push(compact_home(&snapshot.parent));
    if let Some(message) = snapshot.message.as_deref()
        && snapshot.status.is_finished()
        && snapshot.status != JobStatus::Succeeded
    {
        parts.push(message.to_owned());
    }
    parts.join(" · ")
}

fn compact_home(path: &std::path::Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "Home".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {:02}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

fn item_count(count: usize) -> String {
    format!("{count} {}", plural(count, "item", "items"))
}

fn plural(count: usize, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 { singular } else { plural }
}

pub(crate) fn job_icon(snapshot: &JobSnapshot) -> &'static str {
    action_icon(snapshot.icon.as_deref())
}

#[cfg(test)]
mod tests;
