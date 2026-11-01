// SPDX-License-Identifier: MIT

//! Accessible semantics for browser widgets whose nested content has no useful derived name.

use std::cell::RefCell;

use gtk::{glib, prelude::*};

use crate::model::{EntryKind, FileEntry};

use super::{
    browser_modes::BrowserMode,
    keys::{Keys, bindings},
    shortcut_reference::with_keys,
};

#[cfg(feature = "qa")]
pub(crate) mod qa;

pub(super) fn set_label(widget: &impl IsA<gtk::Accessible>, label: &str) {
    widget.update_property(&[gtk::accessible::Property::Label(label)]);
    #[cfg(feature = "qa")]
    qa::record(widget, |naming| naming.label = Some(label.to_owned()));
}

/// A tooltip is not an accessible name, so an icon-only button states it as its name too.
pub(super) fn set_tooltip_and_label(
    widget: &(impl IsA<gtk::Widget> + IsA<gtk::Accessible>),
    text: &str,
) {
    widget.set_tooltip_text(Some(text));
    set_label(widget, text);
}

thread_local! {
    static OWN_LABELS: RefCell<Vec<glib::WeakRef<gtk::Widget>>> = RefCell::default();
    static STAND_INS: RefCell<Vec<glib::WeakRef<gtk::Widget>>> = RefCell::default();
}

fn holds(list: &RefCell<Vec<glib::WeakRef<gtk::Widget>>>, widget: &gtk::Widget) -> bool {
    let mut list = list.borrow_mut();
    list.retain(|kept| kept.upgrade().is_some());
    list.iter()
        .any(|kept| kept.upgrade().as_ref() == Some(widget))
}

pub(super) fn set_own_label(widget: &impl IsA<gtk::Widget>, label: &str) {
    let widget = widget.upcast_ref::<gtk::Widget>();
    OWN_LABELS.with(|own| {
        if !holds(own, widget) {
            own.borrow_mut().push(widget.downgrade());
        }
    });
    STAND_INS.with(|stand_ins| {
        stand_ins
            .borrow_mut()
            .retain(|kept| kept.upgrade().as_ref() != Some(widget));
    });
    set_label(widget, label);
}

pub(super) fn set_stand_in_label(widget: &impl IsA<gtk::Widget>, label: &str) {
    let widget = widget.upcast_ref::<gtk::Widget>();
    if OWN_LABELS.with(|own| holds(own, widget)) {
        return;
    }
    STAND_INS.with(|stand_ins| {
        if !holds(stand_ins, widget) {
            stand_ins.borrow_mut().push(widget.downgrade());
        }
    });
    set_label(widget, label);
}

pub(super) fn clear_stand_in_label(widget: &impl IsA<gtk::Widget>) {
    let widget = widget.upcast_ref::<gtk::Widget>();
    let wore_one = STAND_INS.with(|stand_ins| {
        let held = holds(stand_ins, widget);
        stand_ins
            .borrow_mut()
            .retain(|kept| kept.upgrade().as_ref() != Some(widget));
        held
    });
    if wore_one {
        widget.reset_property(gtk::AccessibleProperty::Label);
        #[cfg(feature = "qa")]
        qa::record(widget, |naming| naming.label = None);
    }
}

pub(super) fn set_description(widget: &impl IsA<gtk::Accessible>, description: Option<&str>) {
    widget.update_property(&[gtk::accessible::Property::Description(
        description.unwrap_or(""),
    )]);
    #[cfg(feature = "qa")]
    qa::record(widget, |naming| {
        naming.description = description.map(str::to_owned);
    });
}

fn tristate(value: bool) -> gtk::AccessibleTristate {
    if value {
        gtk::AccessibleTristate::True
    } else {
        gtk::AccessibleTristate::False
    }
}

pub(super) fn set_checked(widget: &impl IsA<gtk::Accessible>, checked: bool) {
    widget.update_state(&[gtk::accessible::State::Checked(tristate(checked))]);
    #[cfg(feature = "qa")]
    qa::record(widget, |naming| naming.checked = Some(checked));
}

pub(super) fn set_labelled_by(widget: &impl IsA<gtk::Accessible>, labels: &[&gtk::Widget]) {
    let related: Vec<&gtk::Accessible> = labels.iter().map(|label| label.upcast_ref()).collect();
    widget.update_relation(&[gtk::accessible::Relation::LabelledBy(&related)]);
    #[cfg(feature = "qa")]
    qa::record_labelled_by(widget, labels);
}

pub(super) fn clear_labelled_by(widget: &impl IsA<gtk::Accessible>) {
    widget.reset_relation(gtk::AccessibleRelation::LabelledBy);
    #[cfg(feature = "qa")]
    qa::record_labelled_by_reset(widget);
}

pub(super) fn set_selected(widget: &impl IsA<gtk::Accessible>, selected: bool) {
    widget.update_state(&[gtk::accessible::State::Selected(Some(selected))]);
    #[cfg(feature = "qa")]
    qa::record(widget, |naming| naming.selected = Some(selected));
}

pub(super) fn set_pressed(widget: &impl IsA<gtk::Accessible>, pressed: bool) {
    widget.update_state(&[gtk::accessible::State::Pressed(tristate(pressed))]);
    #[cfg(feature = "qa")]
    qa::record(widget, |naming| naming.pressed = Some(pressed));
}

pub(super) fn set_busy(widget: &impl IsA<gtk::Accessible>, busy: bool) {
    widget.update_state(&[gtk::accessible::State::Busy(busy)]);
    #[cfg(feature = "qa")]
    qa::record(widget, |naming| naming.busy = Some(busy));
}

/// Option rows show their chosen state by toggling a check icon from several
/// places; mirroring the icon keeps the accessible state from drifting. The
/// icon's own `visible` property is read because the closed popover hides
/// every option's ancestors.
pub(super) fn sync_checked_with_icon(option: &impl IsA<gtk::Widget>, check: &gtk::Image) {
    let option = option.upcast_ref::<gtk::Widget>();
    set_checked(option, check.get_visible());
    let option = option.downgrade();
    check.connect_visible_notify(move |check| {
        if let Some(option) = option.upgrade() {
            set_checked(&option, check.get_visible());
        }
    });
}

/// Location controls require explicit accessible names; the icon-only buttons
/// also show them as tooltips.
pub(super) fn describe_location_controls(
    entry: &gtk::Entry,
    confirm: &gtk::Button,
    cancel: &gtk::Button,
) {
    use super::shortcut_reference::follow_keys;
    follow_keys(entry, |entry| {
        set_label(
            entry,
            &with_keys("Location (%{keys})", Keys::One(bindings::LOCATION.shown())),
        );
    });
    for (button, message, key) in [
        (confirm, "Navigate (%{keys})", bindings::ENTER),
        (cancel, "Cancel (%{keys})", bindings::ESCAPE),
    ] {
        follow_keys(button, move |button| {
            let label = with_keys(message, Keys::One(key.shown()));
            set_label(button, &label);
            button.set_tooltip_text(Some(&label));
        });
    }
}

/// The name belongs on the list item rather than on the row content: the item
/// is the widget carrying the `list item` / `table cell` accessible role and
/// the selected and focused states.
pub(super) fn describe_entry(item: &gtk::ListItem, display_name: &str, entry: Option<&FileEntry>) {
    let description = entry.map_or_else(|| crate::i18n::tr("Entry"), entry_kind_name);
    item.set_accessible_label(display_name);
    item.set_accessible_description(&description);
    #[cfg(feature = "qa")]
    qa::record(item, |naming| {
        *naming = qa::Naming {
            label: Some(display_name.to_owned()),
            description: Some(description),
            mark: Some(qa::Mark::Entry),
            checked: naming.checked,
            pressed: naming.pressed,
            selected: naming.selected,
            busy: naming.busy,
        };
    });
}

/// A plain `GtkBox` has the `generic` accessible role, and ARIA forbids naming
/// those, so GTK silently drops any label set on one.
pub(super) fn pane_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .accessible_role(gtk::AccessibleRole::Group)
        .build()
}

pub(super) fn dialog_box(title: &str) -> gtk::Box {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .accessible_role(gtk::AccessibleRole::Dialog)
        .build();
    content.update_property(&[gtk::accessible::Property::Label(title)]);
    content.update_state(&[gtk::accessible::State::Hidden(false)]);
    #[cfg(feature = "qa")]
    qa::record(&content, |naming| {
        naming.label = Some(title.to_owned());
        naming.mark = Some(qa::Mark::Dialog);
    });
    content
}

/// The pane, not the entry list, carries this: an empty directory replaces its
/// list with a placeholder, and the pane has to stay identifiable either way.
pub(super) fn describe_pane(pane: &impl IsA<gtk::Accessible>, directory: &str, mode: BrowserMode) {
    let description = view_name(mode);
    pane.update_property(&[
        gtk::accessible::Property::Label(directory),
        gtk::accessible::Property::Description(&description),
    ]);
    #[cfg(feature = "qa")]
    qa::record(pane, |naming| {
        *naming = qa::Naming {
            label: Some(directory.to_owned()),
            description: Some(description),
            mark: Some(qa::Mark::Pane(match mode {
                BrowserMode::Columns => "Columns",
                BrowserMode::Icons => "Icons",
                BrowserMode::List => "List",
            })),
            checked: naming.checked,
            pressed: naming.pressed,
            selected: naming.selected,
            busy: naming.busy,
        };
    });
}

pub(super) const ENTRY_CONTAINER_DESCRIPTION: &str = "Files";

pub(super) fn describe_entry_container(container: &impl IsA<gtk::Accessible>, directory: &str) {
    let description = crate::i18n::tr(ENTRY_CONTAINER_DESCRIPTION);
    container.update_property(&[
        gtk::accessible::Property::Label(directory),
        gtk::accessible::Property::Description(&description),
    ]);
    #[cfg(feature = "qa")]
    qa::record(container, |naming| {
        *naming = qa::Naming {
            label: Some(directory.to_owned()),
            description: Some(description),
            mark: Some(qa::Mark::EntryContainer),
            checked: naming.checked,
            pressed: naming.pressed,
            selected: naming.selected,
            busy: naming.busy,
        };
    });
}

pub(super) const LOADING_SURFACE_DESCRIPTION: &str = "Loading";

/// The pane surface takes keyboard focus while a directory is empty, unreadable or
/// loading; it is named after the directory and described by what it shows.
pub(super) fn describe_pane_surface(
    surface: &impl IsA<gtk::Accessible>,
    directory: &str,
    status: &str,
) {
    surface.update_property(&[
        gtk::accessible::Property::Label(directory),
        gtk::accessible::Property::Description(status),
    ]);
    #[cfg(feature = "qa")]
    qa::record(surface, |naming| {
        *naming = qa::Naming {
            label: Some(directory.to_owned()),
            description: Some(status.to_owned()),
            mark: Some(qa::Mark::PaneSurface),
            checked: naming.checked,
            pressed: naming.pressed,
            selected: naming.selected,
            busy: naming.busy,
        };
    });
}

pub(super) fn menu_item_button() -> gtk::Button {
    gtk::Button::builder()
        .accessible_role(gtk::AccessibleRole::MenuItem)
        .has_frame(false)
        .build()
}

pub(super) fn menu_box() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .accessible_role(gtk::AccessibleRole::Menu)
        .build()
}

/// Names a menu item after its action and announces its accelerator
/// separately, so the shortcut text does not become part of the name.
pub(super) fn describe_menu_item(
    button: &impl IsA<gtk::Accessible>,
    label: &str,
    accelerator: &str,
) {
    button.update_property(&[
        gtk::accessible::Property::Label(label),
        gtk::accessible::Property::Description(accelerator),
    ]);
    #[cfg(feature = "qa")]
    qa::record(button, |naming| {
        *naming = qa::Naming {
            label: Some(label.to_owned()),
            description: Some(accelerator.to_owned()),
            mark: Some(qa::Mark::MenuItem),
            checked: naming.checked,
            pressed: naming.pressed,
            selected: naming.selected,
            busy: naming.busy,
        };
    });
}

pub(super) fn view_name(mode: BrowserMode) -> String {
    crate::i18n::tr(match mode {
        BrowserMode::Columns => "Columns view",
        BrowserMode::Icons => "Icons view",
        BrowserMode::List => "List view",
    })
}

fn entry_kind_name(entry: &FileEntry) -> String {
    crate::i18n::tr(match entry.kind {
        EntryKind::Directory if entry.is_locked_folder() => {
            "Folder that requires administrator access"
        }
        EntryKind::DirectorySymbolicLink if entry.is_locked_folder() => {
            "Folder link that requires administrator access"
        }
        EntryKind::Directory => "Folder",
        EntryKind::DirectorySymbolicLink => "Folder link",
        EntryKind::File => "File",
        EntryKind::FileSymbolicLink => "File link",
        EntryKind::SymbolicLink => "Broken link",
        EntryKind::Other => "Other",
    })
}

#[cfg(test)]
mod tests;
