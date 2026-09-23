// SPDX-License-Identifier: MIT

//! An edit is a lease on a bound entry, not on a row position. Presentations hand out typed
//! widgets and optional reveal/cleanup hooks. Rebinding to another identity, unbinding, dropping
//! the owner or cancelling revokes the lease. Submission is consumer policy; filesystem work
//! and its pending/error reconciliation remain in the browser's operation coordinator.

use crate::{
    model::{FileEntry, Location},
    services::validate_basename,
};
use gtk::{glib, prelude::*};
use std::{cell::RefCell, rc::Rc};

type Cancel = Rc<dyn Fn()>;
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
}

impl From<EditWidgets> for EditTarget {
    fn from(widgets: EditWidgets) -> Self {
        Self {
            widgets,
            reveal: None,
            finish: None,
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
}

impl Drop for ActiveEdit {
    fn drop(&mut self) {
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
        self.field.set_tooltip_text(None);
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
) -> Option<(FileEntry, String)> {
    if !field.is_sensitive()
        || !active
            .borrow()
            .as_ref()
            .is_some_and(|edit| edit.field == *field)
    {
        return None;
    }
    let edit = active.take()?;
    let result = (edit.entry.clone(), field.text().to_string());
    drop(edit);
    Some(result)
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
    field.set_tooltip_text(None);
    let changed = field.connect_changed(|field| {
        update_basename_validation(field);
    });
    let activate = submit.clone();
    let activated = field.connect_activate(move |field| activate(field));
    let focus = gtk::EventControllerFocus::new();
    let focus_handler = focus.connect_leave(move |focus| {
        if let Some(field) = focus.widget().and_downcast::<gtk::Entry>() {
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

pub(super) fn rename_stem_end(name: &str) -> i32 {
    let end = name
        .rfind('.')
        .filter(|position| *position > 0)
        .unwrap_or(name.len());
    name[..end].chars().count().min(i32::MAX as usize) as i32
}

// Empty fields are an ordinary editing state, although they cannot be submitted.
pub(super) fn basename_field_error(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        None
    } else {
        validate_basename(name).err()
    }
}

pub(in crate::ui) fn update_basename_validation(field: &gtk::Entry) -> bool {
    let text = field.text();
    match basename_field_error(text.as_str()) {
        None => {
            field.remove_css_class("error");
            field.set_tooltip_text(None);
            !text.is_empty()
        }
        Some(message) => {
            field.add_css_class("error");
            field.set_tooltip_text(Some(message));
            false
        }
    }
}
