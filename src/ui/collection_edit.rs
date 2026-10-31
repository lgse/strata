// SPDX-License-Identifier: MIT

//! An edit is a lease on a bound entry, not on a row position. Presentations hand out typed
//! widgets and optional reveal/cleanup hooks. Rebinding to another identity, unbinding, dropping
//! the owner or cancelling revokes the lease. Submission is consumer policy; filesystem work
//! and its pending/error reconciliation remain in the browser's operation coordinator.

use crate::{
    model::{FileEntry, Location},
    services::{NewEntryPath, validate_basename},
};
use gtk::{glib, prelude::*};
use std::{cell::RefCell, rc::Rc};

type Cancel = Rc<dyn Fn()>;

thread_local! {
    static ITEM_EDITS: RefCell<std::collections::HashMap<usize, (glib::WeakRef<glib::Object>, EditWidgets)>> =
        RefCell::new(std::collections::HashMap::new());
}

pub(super) fn register_item_edit(item: &impl IsA<glib::Object>, edit: &EditWidgets) {
    let item = item.upcast_ref::<glib::Object>();
    ITEM_EDITS.with_borrow_mut(|edits| {
        edits.insert(item.as_ptr() as usize, (item.downgrade(), edit.clone()));
    });
}

pub(super) fn item_edit(item: &impl IsA<glib::Object>) -> Option<EditWidgets> {
    let item = item.upcast_ref::<glib::Object>();
    ITEM_EDITS.with_borrow(|edits| {
        edits
            .get(&(item.as_ptr() as usize))
            .filter(|(owner, _)| owner.upgrade().as_ref() == Some(item))
            .map(|(_, edit)| edit.clone())
    })
}

pub(super) fn forget_item_edit(item: &glib::Object) {
    ITEM_EDITS.with_borrow_mut(|edits| {
        edits.remove(&(item.as_ptr() as usize));
    });
}
pub(super) type ActiveEdits = Rc<RefCell<Option<ActiveEdit>>>;
pub(super) type Submit = Rc<dyn Fn(&gtk::Entry)>;
pub(super) type Reveal = Rc<dyn Fn(&gtk::Entry)>;

#[derive(Clone)]
pub(super) struct EditWidgets {
    pub(super) field: gtk::Entry,
    pub(super) display: gtk::Widget,
    identity: Rc<RefCell<Option<Location>>>,
    cancel: Rc<RefCell<Option<Cancel>>>,
}

impl EditWidgets {
    pub(super) fn new(field: &gtk::Entry, display: &impl IsA<gtk::Widget>) -> Self {
        Self {
            field: field.clone(),
            display: display.clone().upcast(),
            identity: Rc::default(),
            cancel: Rc::default(),
        }
    }

    pub(super) fn bind(&self, location: &Location) {
        if self.identity.borrow().as_ref() != Some(location) {
            self.unbind();
        }
        self.identity.replace(Some(location.clone()));
    }

    pub(super) fn unbind(&self) {
        let cancel = self.cancel.borrow().clone();
        if let Some(cancel) = cancel {
            cancel();
        }
        self.identity.take();
    }

    pub(super) fn is_editing(&self) -> bool {
        self.cancel.borrow().is_some()
    }
}

pub(super) struct EditTarget {
    pub(super) widgets: EditWidgets,
    pub(super) reveal: Option<Reveal>,
    pub(super) finish: Option<Rc<dyn Fn()>>,
    pub(super) cancelled: Option<Rc<dyn Fn()>>,
    /// The entry was just created, so a local one may be named with a path.
    pub(super) new_item: bool,
}

impl From<EditWidgets> for EditTarget {
    fn from(widgets: EditWidgets) -> Self {
        Self {
            widgets,
            reveal: None,
            finish: None,
            cancelled: None,
            new_item: false,
        }
    }
}

pub(super) struct ActiveEdit {
    pub(super) entry: FileEntry,
    pub(super) field: gtk::Entry,
    target: EditTarget,
    handlers: Vec<glib::SignalHandlerId>,
    focus: gtk::EventControllerFocus,
    focus_handler: Option<glib::SignalHandlerId>,
    tick: Option<gtk::TickCallbackId>,
    submitted: bool,
    allows_path: bool,
}

pub(super) struct Submission {
    pub(super) entry: FileEntry,
    pub(super) name: String,
    /// The name may be a relative path; see [`NewEntryPath`].
    pub(super) allows_path: bool,
}

impl Drop for ActiveEdit {
    fn drop(&mut self) {
        if !self.submitted
            && let Some(on_cancel) = &self.target.cancelled
        {
            on_cancel();
        }
        self.target.widgets.cancel.take();
        for handler in self.handlers.drain(..) {
            self.field.disconnect(handler);
        }
        if let Some(handler) = self.focus_handler.take() {
            self.focus.disconnect(handler);
        }
        // GTK walks the controller list during Tab focus crossing. Retire the handler now,
        // but detach its controller only after that walk has unwound.
        let field = self.field.downgrade();
        let focus = self.focus.clone();
        glib::idle_add_local_once(move || {
            if let Some(field) = field.upgrade() {
                field.remove_controller(&focus);
            }
        });
        if let Some(tick) = self.tick.take() {
            tick.remove();
        }
        self.field.set_visible(false);
        self.field.set_sensitive(true);
        self.field.remove_css_class("error");
        crate::ui::accessibility::set_description(&self.field, None);
        self.target.widgets.display.set_visible(true);
        if let Some(finish) = &self.target.finish {
            finish();
        }
    }
}

pub(super) fn cancel(active: &RefCell<Option<ActiveEdit>>) -> bool {
    let edit = active.take();
    let cancelled = edit.is_some();
    drop(edit);
    cancelled
}

pub(super) fn take_submission(
    active: &RefCell<Option<ActiveEdit>>,
    field: &gtk::Entry,
) -> Option<Submission> {
    if !field.is_sensitive()
        || !active
            .borrow()
            .as_ref()
            .is_some_and(|edit| edit.field == *field)
    {
        return None;
    }
    let mut edit = active.take()?;
    edit.submitted = true;
    let submission = Submission {
        entry: edit.entry.clone(),
        name: field.text().to_string(),
        allows_path: edit.allows_path,
    };
    drop(edit);
    Some(submission)
}

pub(super) fn begin(
    active: &ActiveEdits,
    entry: FileEntry,
    target: EditTarget,
    submit: Submit,
) -> bool {
    cancel(active);
    if target.widgets.identity.borrow().as_ref() != Some(&entry.location) {
        return false;
    }
    let field = target.widgets.field.clone();
    field.set_text(&entry.display_name);
    field.set_sensitive(true);
    field.remove_css_class("error");
    crate::ui::accessibility::set_description(&field, None);
    let allows_path = target.new_item && entry.location.native_path().is_some();
    let path_kind = allows_path.then_some(entry.is_directory());
    let changed = field.connect_changed(move |field| {
        update_name_validation(field, path_kind);
    });
    let activate = submit.clone();
    let activated = field.connect_activate(move |field| activate(field));
    let focus = gtk::EventControllerFocus::new();
    let focus_handler = focus.connect_leave(move |focus| {
        if let Some(field) = focus.widget().and_downcast::<gtk::Entry>()
            && !kept_focus_in_inactive_window(&field)
        {
            submit(&field);
        }
    });
    field.add_controller(focus.clone());
    let weak = Rc::downgrade(active);
    let weak_field = field.downgrade();
    let cancel: Cancel = Rc::new(move || {
        if let (Some(active), Some(field)) = (weak.upgrade(), weak_field.upgrade()) {
            let matches = active
                .borrow()
                .as_ref()
                .is_some_and(|edit| edit.field == field);
            if matches {
                self::cancel(&active);
            }
        }
    });
    target.widgets.cancel.replace(Some(cancel.clone()));
    let unmapped = field.connect_unmap(move |_| cancel());
    let tick = target.reveal.as_ref().map(|reveal| {
        let reveal = reveal.clone();
        field.add_tick_callback(move |field, _| {
            reveal(field);
            glib::ControlFlow::Continue
        })
    });
    target.widgets.display.set_visible(false);
    field.set_visible(true);
    let end = if entry.is_directory() {
        -1
    } else {
        rename_stem_end(&entry.display_name)
    };
    active.replace(Some(ActiveEdit {
        entry,
        field: field.clone(),
        target,
        handlers: vec![changed, activated, unmapped],
        focus,
        focus_handler: Some(focus_handler),
        tick,
        submitted: false,
        allows_path,
    }));
    field.grab_focus();
    field.select_region(0, end);
    let weak = Rc::downgrade(active);
    let weak_field = field.downgrade();
    glib::idle_add_local_once(move || {
        if let (Some(active), Some(field)) = (weak.upgrade(), weak_field.upgrade())
            && active
                .borrow()
                .as_ref()
                .is_some_and(|edit| edit.field == field)
            && field.is_mapped()
        {
            field.grab_focus_without_selecting();
        }
    });
    true
}

fn kept_focus_in_inactive_window(field: &gtk::Entry) -> bool {
    field
        .root()
        .and_downcast::<gtk::Window>()
        .is_some_and(|window| {
            !window.is_active()
                && GtkWindowExt::focus(&window).is_some_and(|focus| {
                    focus == *field.upcast_ref::<gtk::Widget>() || focus.is_ancestor(field)
                })
        })
}

pub(super) fn rename_stem_end(name: &str) -> i32 {
    let end = name
        .rfind('.')
        .filter(|position| *position > 0)
        .unwrap_or(name.len());
    name[..end].chars().count().min(i32::MAX as usize) as i32
}

/// Empty fields are an ordinary editing state, although they cannot be submitted.
/// `path_kind` is whether a new item that may be named with a path is a folder.
pub(super) fn name_field_error(name: &str, path_kind: Option<bool>) -> Option<&'static str> {
    match path_kind {
        _ if name.is_empty() => None,
        Some(is_directory) => NewEntryPath::parse(name, is_directory).err(),
        None => validate_basename(name).err(),
    }
}

pub(in crate::ui) fn update_basename_validation(field: &gtk::Entry) -> bool {
    update_name_validation(field, None)
}

fn update_name_validation(field: &gtk::Entry, path_kind: Option<bool>) -> bool {
    let text = field.text();
    match name_field_error(text.as_str(), path_kind) {
        None => {
            field.remove_css_class("error");
            crate::ui::accessibility::set_description(field, None);
            !text.is_empty()
        }
        Some(message) => {
            field.add_css_class("error");
            crate::ui::accessibility::set_description(field, Some(&crate::i18n::tr(message)));
            false
        }
    }
}
