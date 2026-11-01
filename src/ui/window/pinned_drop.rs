// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::{cell::RefCell, future::Future, rc::Rc, time::Duration};

use gtk::{gdk, glib, prelude::*};

use super::SidebarState;
use crate::{
    model::Location,
    ui::browser::{active_drag_snapshot, paths::is_trash_location},
};

type Folder = (Location, String);
type FolderResult = Option<Rc<[Folder]>>;
type Completion = Box<dyn FnOnce(FolderResult)>;

const VALIDATION_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct SidebarDrag {
    source: glib::WeakRef<gtk::DragSource>,
    folder: Folder,
    pinned: bool,
}

thread_local! {
    static ACTIVE_DRAG: RefCell<Option<SidebarDrag>> = const { RefCell::new(None) };
}

pub(super) fn start_drag(source: &gtk::DragSource, folder: Folder, pinned: bool) {
    ACTIVE_DRAG.with(|active| {
        active.replace(Some(SidebarDrag {
            source: source.downgrade(),
            folder,
            pinned,
        }))
    });
}

pub(super) fn clear_reorder() {
    ACTIVE_DRAG.with(|active| active.take());
}

pub(in crate::ui::window) fn active_drag_source() -> Option<gtk::DragSource> {
    ACTIVE_DRAG.with(|active| active.borrow().as_ref()?.source.upgrade())
}

fn drag_source(drop: &gdk::Drop) -> Option<SidebarDrag> {
    let drag = drop.drag()?;
    ACTIVE_DRAG.with(|active| {
        active
            .borrow()
            .as_ref()
            .filter(|source| {
                source
                    .source
                    .upgrade()
                    .and_then(|source| source.drag())
                    .as_ref()
                    == Some(&drag)
            })
            .cloned()
    })
}

pub(in crate::ui::window) fn dragged_location(drop: &gdk::Drop) -> Option<Location> {
    drag_source(drop).map(|source| source.folder.0)
}

fn reorder_source(drop: &gdk::Drop) -> Option<Folder> {
    drag_source(drop)
        .filter(|source| source.pinned)
        .map(|source| source.folder)
}

#[derive(Clone)]
enum ValidationResult {
    Pending,
    Complete(FolderResult),
}

struct FolderValidation {
    result: RefCell<ValidationResult>,
    completions: RefCell<Vec<Completion>>,
    job: RefCell<Option<glib::JoinHandle<()>>>,
}

impl FolderValidation {
    fn ready(result: FolderResult) -> Rc<Self> {
        Rc::new(Self {
            result: RefCell::new(ValidationResult::Complete(result)),
            completions: RefCell::new(Vec::new()),
            job: RefCell::new(None),
        })
    }

    fn start(query: impl Future<Output = FolderResult> + 'static, timeout: Duration) -> Rc<Self> {
        let validation = Self::ready(None);
        validation.result.replace(ValidationResult::Pending);
        let weak = Rc::downgrade(&validation);
        let job = glib::MainContext::ref_thread_default().spawn_local(async move {
            let result = glib::future_with_timeout(timeout, query)
                .await
                .ok()
                .flatten();
            if let Some(validation) = weak.upgrade() {
                validation.finish(result);
            }
        });
        validation.job.replace(Some(job));
        validation
    }

    fn finish(&self, result: FolderResult) {
        if matches!(*self.result.borrow(), ValidationResult::Complete(_)) {
            return;
        }
        self.result
            .replace(ValidationResult::Complete(result.clone()));
        let completions = self.completions.take();
        for completion in completions {
            completion(result.clone());
        }
    }

    fn on_ready(&self, completion: impl FnOnce(FolderResult) + 'static) {
        let result = self.result.borrow().clone();
        match result {
            ValidationResult::Pending => self.completions.borrow_mut().push(Box::new(completion)),
            ValidationResult::Complete(result) => completion(result),
        }
    }

    fn action(&self, preferred: gdk::DragAction) -> gdk::DragAction {
        match &*self.result.borrow() {
            ValidationResult::Complete(None) => gdk::DragAction::empty(),
            // Retain the pin intent if released before validation finishes.
            ValidationResult::Pending | ValidationResult::Complete(Some(_)) => preferred,
        }
    }
}

impl Drop for FolderValidation {
    fn drop(&mut self) {
        if let Some(job) = self.job.get_mut().take() {
            job.abort();
        }
        self.finish(None);
    }
}

#[derive(Default)]
pub(super) struct FolderDragCache {
    current: RefCell<Option<(glib::WeakRef<gdk::Drop>, Rc<FolderValidation>)>>,
    local_only: bool,
}

impl FolderDragCache {
    pub(super) fn new(local_only: bool) -> Self {
        Self {
            local_only,
            ..Self::default()
        }
    }

    fn validation(&self, drop: &gdk::Drop) -> Rc<FolderValidation> {
        let local_only = self.local_only;
        if let Some((cached_drop, validation)) = self.current.borrow().as_ref()
            && cached_drop.upgrade().as_ref() == Some(drop)
        {
            return validation.clone();
        }
        let validation = if let Some(folder) = reorder_source(drop) {
            let allowed = can_pin(&folder.0, local_only);
            FolderValidation::ready(allowed.then(|| vec![folder].into()))
        } else if let Some(snapshot) = active_drag_snapshot(drop) {
            FolderValidation::ready(snapshot.folders.filter(|folders| {
                !folders.is_empty()
                    && folders
                        .iter()
                        .all(|(location, _)| can_pin(location, local_only))
            }))
        } else {
            let drop = drop.clone();
            FolderValidation::start(
                async move {
                    let value = drop
                        .read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT)
                        .await
                        .ok()?;
                    let files = value.get::<gdk::FileList>().ok()?;
                    let locations = files
                        .files()
                        .iter()
                        .map(crate::adapters::location_for_file)
                        .collect::<Option<Vec<_>>>()?;
                    if !locations
                        .iter()
                        .all(|location| can_pin(location, local_only))
                    {
                        return None;
                    }
                    crate::adapters::directory_validation::folder_names(locations)
                        .await
                        .map(Into::into)
                },
                VALIDATION_TIMEOUT,
            )
        };
        // Drop the old request outside the borrow: cancellation can complete a pending drop.
        let old = self
            .current
            .replace(Some((drop.downgrade(), validation.clone())));
        std::mem::drop(old);
        validation
    }
}

/// Checked before probing: trash cannot be pinned, and GVfs would fetch web links on hover.
fn can_pin(location: &Location, local_only: bool) -> bool {
    (!local_only || location.native_path().is_some())
        && !is_trash_location(location)
        && !location.uri_value().is_some_and(|uri| {
            glib::Uri::parse_scheme(uri).is_some_and(|scheme| {
                scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
            })
        })
}

fn pin_action(drop: &gdk::Drop) -> gdk::DragAction {
    if drop.actions().contains(gdk::DragAction::MOVE) && reorder_source(drop).is_some() {
        gdk::DragAction::MOVE
    } else if drop.actions().contains(gdk::DragAction::COPY) {
        gdk::DragAction::COPY
    } else if drop.actions().contains(gdk::DragAction::MOVE) && active_drag_snapshot(drop).is_some()
    {
        // Our sources do not delete data on drag-end. Foreign MOVE completions
        // may request deletion, so only provider-matched internal drags can use it.
        gdk::DragAction::MOVE
    } else {
        gdk::DragAction::empty()
    }
}

struct PinDropSite {
    widget: glib::WeakRef<gtk::Box>,
    hover_drop: RefCell<Option<gdk::Drop>>,
    feedback: Rc<super::sidebar_drop_feedback::SidebarDropFeedback>,
}

impl PinDropSite {
    fn enter(self: &Rc<Self>, drop: &gdk::Drop, cache: &FolderDragCache) -> gdk::DragAction {
        self.leave();
        self.hover_drop.replace(Some(drop.clone()));
        if let Some(widget) = self.widget.upgrade() {
            self.feedback.enter(widget.upcast_ref());
        }
        let validation = cache.validation(drop);
        let action = validation.action(pin_action(drop));
        let site = Rc::downgrade(self);
        let drop = drop.clone();
        validation.on_ready(move |locations| {
            let Some(site) = site.upgrade() else { return };
            if site.hover_drop.borrow().as_ref() != Some(&drop) {
                return;
            }
            if let Some(widget) = site.widget.upgrade()
                && widget.root().is_some()
                && site.feedback.is_current(widget.upcast_ref())
            {
                let action = if locations.is_some() {
                    pin_action(&drop)
                } else {
                    gdk::DragAction::empty()
                };
                if action.is_empty() {
                    site.feedback.leave(widget.upcast_ref());
                } else {
                    site.feedback.pin(widget.upcast_ref());
                }
                drop.status(action, action);
            }
        });
        action
    }

    fn leave(&self) {
        self.hover_drop.take();
        if let Some(widget) = self.widget.upgrade() {
            self.feedback.leave(widget.upcast_ref());
        }
    }
}

pub(super) fn append_gap(
    state: &Rc<SidebarState>,
    after: Option<Location>,
    before: Option<Location>,
    cache: &Rc<FolderDragCache>,
) {
    let gap = super::sidebar_drop_feedback::row_gap(Some(&crate::i18n::tr("PINNED")));
    gap.add_css_class("sidebar-pin-gap");
    if after.is_none() && before.is_none() {
        gap.add_css_class("empty");
    }
    let target = gtk::DropTargetAsync::new(
        Some(
            gdk::ContentFormats::for_type(gdk::FileList::static_type())
                .union(&gdk::ContentFormats::for_type(String::static_type())),
        ),
        gdk::DragAction::COPY | gdk::DragAction::MOVE,
    );
    target.connect_accept(|_, drop| {
        !pin_action(drop).is_empty()
            && (reorder_source(drop).is_some()
                || drop
                    .formats()
                    .union_deserialize_types()
                    .contains_type(gdk::FileList::static_type()))
    });
    let site = Rc::new(PinDropSite {
        widget: gap.downgrade(),
        hover_drop: RefCell::new(None),
        feedback: state.drop_feedback.clone(),
    });
    let enter_site = site.clone();
    let enter_cache = cache.clone();
    target.connect_drag_enter(move |_, drop, _, _| enter_site.enter(drop, &enter_cache));
    let motion_cache = cache.clone();
    target.connect_drag_motion(move |_, drop, _, _| {
        motion_cache.validation(drop).action(pin_action(drop))
    });
    let leave_site = site.clone();
    target.connect_drag_leave(move |_, _| leave_site.leave());
    let weak_state = Rc::downgrade(state);
    let drop_cache = cache.clone();
    let widget = gap.downgrade();
    target.connect_drop(move |_, drop, _, _| {
        let action = pin_action(drop);
        if action.is_empty() {
            return false;
        }
        site.leave();
        let validation = drop_cache.validation(drop);
        let state = weak_state.clone();
        let widget = widget.clone();
        let after = after.clone();
        let before = before.clone();
        let reorder = reorder_source(drop).is_some();
        let drop = drop.clone();
        validation.on_ready(move |locations| {
            let finish = move |success: bool| {
                drop.finish(if success {
                    action
                } else {
                    gdk::DragAction::empty()
                });
            };
            match (locations, state.upgrade(), widget.upgrade()) {
                (Some(locations), Some(state), Some(widget)) if widget.root().is_some() => {
                    let gap = (after.as_ref(), before.as_ref());
                    match (reorder, locations.first()) {
                        (true, Some((source, _))) => state.reorder_pinned_into(source, gap, finish),
                        (true, None) => finish(false),
                        (false, _) => state.pin_folders_into(&locations, gap, finish),
                    }
                }
                _ => finish(false),
            }
        });
        true
    });
    gap.add_controller(target);
    state.widget.append(&gap);
}
