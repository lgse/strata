// SPDX-License-Identifier: MIT

//! Finder-style batch rename dialog for multi-selections.
//!
//! These methods live on [`ViewState`] and start work through the shared
//! [`crate::app::Browser`] controller; they must not create a separate rename
//! pipeline.
//!
//! # Entry points
//!
//! - [`ViewState::show_batch_rename_dialog`]

use crate::model::FileEntry;
use crate::services::{BatchRenameMode, FormatStyle, plan_batch_rename};
use crate::ui::browser::ViewState;
use crate::ui::browser::entry::{entry_kind_summary, item_count_label};
use crate::ui::controls::{form_entry, form_label, modal_layout, segmented_control};
use crate::ui::modal::{
    ModalHost, dismiss_modal_layer, modal_layer, show_error_dialog, submit_on_enter,
};
use gtk::{glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, Eq, PartialEq)]
enum RenameDialogMode {
    Replace,
    Add,
    Format,
}

impl ViewState {
    /// Opens the batch rename dialog for the selected `entries`.
    ///
    /// Returns immediately for fewer than two entries; single renames use the
    /// inline editor. Planned names pair back with `entries` in order, so the
    /// selection's view order drives counter numbering.
    pub(super) fn show_batch_rename_dialog(self: &Rc<Self>, entries: Vec<FileEntry>) {
        if entries.len() < 2 {
            return;
        }
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };
        let title = format!("Rename {}", item_count_label(entries.len()));
        let layout = modal_layout(
            crate::assets::icons::PENCIL,
            &title,
            &entry_kind_summary(&entries),
            "Rename",
        );

        let mode = Rc::new(Cell::new(RenameDialogMode::Replace));
        let (mode_control, mode_options) =
            segmented_control(&["Replace Text", "Add Text", "Format"], 0);
        layout.body.append(&mode_control);

        let replace_box = batch_rename_section();
        let find_label = form_label("Find");
        let find_entry = form_entry();
        let replace_label = form_label("Replace with");
        let replace_entry = form_entry();
        replace_box.append(&find_label);
        replace_box.append(&find_entry);
        replace_box.append(&replace_label);
        replace_box.append(&replace_entry);

        let add_box = batch_rename_section();
        let text_label = form_label("Text");
        let text_entry = form_entry();
        let position_label = form_label("Position");
        let (position_control, position_options) =
            segmented_control(&["Before name", "After name"], 1);
        let before_name = Rc::new(Cell::new(false));
        add_box.append(&text_label);
        add_box.append(&text_entry);
        add_box.append(&position_label);
        add_box.append(&position_control);
        add_box.set_visible(false);

        let format_box = batch_rename_section();
        let base_label = form_label("Custom name");
        let base_entry = form_entry();
        let style_label = form_label("Name style");
        let (style_control, style_options) = segmented_control(&["Counter", "Index", "Date"], 0);
        let style = Rc::new(Cell::new(FormatStyle::Counter));
        let start_label = form_label("Start numbers at");
        let start_entry = form_entry();
        start_entry.set_text("1");
        start_entry.set_input_purpose(gtk::InputPurpose::Digits);
        let format_position_label = form_label("Position");
        let (format_position_control, format_position_options) =
            segmented_control(&["After name", "Before name"], 0);
        let format_before = Rc::new(Cell::new(false));
        format_box.append(&base_label);
        format_box.append(&base_entry);
        format_box.append(&style_label);
        format_box.append(&style_control);
        format_box.append(&start_label);
        format_box.append(&start_entry);
        format_box.append(&format_position_label);
        format_box.append(&format_position_control);
        format_box.set_visible(false);

        layout.body.append(&replace_box);
        layout.body.append(&add_box);
        layout.body.append(&format_box);

        let example = form_label("");
        layout.body.append(&example);

        let entries_for_preview = entries.clone();
        let preview = example.clone();
        let find_for_preview = find_entry.clone();
        let replace_for_preview = replace_entry.clone();
        let text_for_preview = text_entry.clone();
        let before_for_preview = before_name.clone();
        let base_for_preview = base_entry.clone();
        let style_for_preview = style.clone();
        let start_for_preview = start_entry.clone();
        let format_before_for_preview = format_before.clone();
        let mode_for_preview = mode.clone();
        let refresh_preview = Rc::new(move || {
            let find = find_for_preview.text();
            let replace_with = replace_for_preview.text();
            let text = text_for_preview.text();
            let base = base_for_preview.text();
            let start = start_for_preview.text();
            let input = RenameDialogInput {
                mode: mode_for_preview.get(),
                find: find.as_str(),
                replace_with: replace_with.as_str(),
                text: text.as_str(),
                before_name: before_for_preview.get(),
                base: base.as_str(),
                style: style_for_preview.get(),
                start: start.as_str(),
                format_before: format_before_for_preview.get(),
            };
            let Some(planned) = plan_preview_name(&entries_for_preview, &input) else {
                preview.set_text("Enter text to preview the new names.");
                return;
            };
            let (old, new) = planned;
            if old == new {
                preview.set_text("Names are unchanged.");
            } else {
                preview.set_text(&format!("{old} → {new}"));
            }
        });
        for field in [
            &find_entry,
            &replace_entry,
            &text_entry,
            &base_entry,
            &start_entry,
        ] {
            let refresh = refresh_preview.clone();
            field.connect_changed(move |_| refresh());
        }

        let replace_for_mode = replace_box.clone();
        let add_for_mode = add_box.clone();
        let format_for_mode = format_box.clone();
        let find_for_mode = find_entry.clone();
        let text_for_mode = text_entry.clone();
        let base_for_mode = base_entry.clone();
        for (option, selected) in mode_options.into_iter().zip([
            RenameDialogMode::Replace,
            RenameDialogMode::Add,
            RenameDialogMode::Format,
        ]) {
            let mode = mode.clone();
            let refresh = refresh_preview.clone();
            let replace_for_mode = replace_for_mode.clone();
            let add_for_mode = add_for_mode.clone();
            let format_for_mode = format_for_mode.clone();
            let find_for_mode = find_for_mode.clone();
            let text_for_mode = text_for_mode.clone();
            let base_for_mode = base_for_mode.clone();
            option.connect_toggled(move |option| {
                if !option.is_active() {
                    return;
                }
                mode.set(selected);
                replace_for_mode.set_visible(selected == RenameDialogMode::Replace);
                add_for_mode.set_visible(selected == RenameDialogMode::Add);
                format_for_mode.set_visible(selected == RenameDialogMode::Format);
                match selected {
                    RenameDialogMode::Replace => {
                        find_for_mode.grab_focus();
                    }
                    RenameDialogMode::Add => {
                        text_for_mode.grab_focus();
                    }
                    RenameDialogMode::Format => {
                        base_for_mode.grab_focus();
                    }
                }
                refresh();
            });
        }
        for (option, before) in position_options.into_iter().zip([true, false]) {
            let before_name = before_name.clone();
            let refresh = refresh_preview.clone();
            option.connect_toggled(move |option| {
                if !option.is_active() {
                    return;
                }
                before_name.set(before);
                refresh();
            });
        }
        for (option, before) in format_position_options.into_iter().zip([false, true]) {
            let format_before = format_before.clone();
            let refresh = refresh_preview.clone();
            option.connect_toggled(move |option| {
                if !option.is_active() {
                    return;
                }
                format_before.set(before);
                refresh();
            });
        }
        for (option, format_style) in style_options.into_iter().zip([
            FormatStyle::Counter,
            FormatStyle::Index,
            FormatStyle::Date,
        ]) {
            let style = style.clone();
            let refresh = refresh_preview.clone();
            option.connect_toggled(move |option| {
                if !option.is_active() {
                    return;
                }
                style.set(format_style);
                refresh();
            });
        }
        refresh_preview();

        let dirty_find = find_entry.clone();
        let dirty_replace = replace_entry.clone();
        let dirty_text = text_entry.clone();
        let dirty_base = base_entry.clone();
        let layer = modal_layer(
            &layout.content,
            &window_overlay,
            blurred_root.clone(),
            Some(Rc::new(move || {
                !dirty_find.text().is_empty()
                    || !dirty_replace.text().is_empty()
                    || !dirty_text.text().is_empty()
                    || !dirty_base.text().is_empty()
            })),
        );
        window_overlay.add_overlay(&layer);
        let dismiss: Rc<dyn Fn()> = Rc::new({
            let layer = layer.clone();
            let overlay = window_overlay.clone();
            let root = blurred_root.clone();
            move || dismiss_modal_layer(&layer, &overlay, root.as_ref())
        });
        let dismiss_for_cancel = dismiss.clone();
        layout.cancel.connect_clicked(move |_| dismiss_for_cancel());
        let dismiss_for_close = dismiss.clone();
        layout.close.connect_clicked(move |_| dismiss_for_close());
        let escape = gtk::EventControllerKey::new();
        let dismiss_for_escape = dismiss.clone();
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                dismiss_for_escape();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        submit_on_enter(&layout.body, &layout.confirm);

        let state = Rc::downgrade(self);
        let overlay_for_error = self.overlay.clone();
        let dismiss_for_confirm = dismiss.clone();
        let find_for_confirm = find_entry.clone();
        let replace_for_confirm = replace_entry.clone();
        let text_for_confirm = text_entry.clone();
        let before_for_confirm = before_name.clone();
        let base_for_confirm = base_entry.clone();
        let style_for_confirm = style.clone();
        let start_for_confirm = start_entry.clone();
        let format_before_for_confirm = format_before.clone();
        let mode_for_confirm = mode.clone();
        layout.confirm.connect_clicked(move |_| {
            let Some(state) = state.upgrade() else {
                return;
            };
            let find = find_for_confirm.text();
            let replace_with = replace_for_confirm.text();
            let text = text_for_confirm.text();
            let base = base_for_confirm.text();
            let start = start_for_confirm.text();
            let input = RenameDialogInput {
                mode: mode_for_confirm.get(),
                find: find.as_str(),
                replace_with: replace_with.as_str(),
                text: text.as_str(),
                before_name: before_for_confirm.get(),
                base: base.as_str(),
                style: style_for_confirm.get(),
                start: start.as_str(),
                format_before: format_before_for_confirm.get(),
            };
            let planned_mode = match dialog_rename_mode(&input) {
                Ok(planned_mode) => planned_mode,
                Err(RenameDialogError::MissingFind) => {
                    flag_required(&find_for_confirm, "Enter the text to find.");
                    return;
                }
                Err(RenameDialogError::MissingText) => {
                    flag_required(&text_for_confirm, "Enter the text to add.");
                    return;
                }
                Err(RenameDialogError::MissingBase) => {
                    flag_required(&base_for_confirm, "Enter a name.");
                    return;
                }
                Err(RenameDialogError::BadStart) => {
                    flag_required(&start_for_confirm, "Enter a number.");
                    return;
                }
            };
            let names: Vec<String> = entries
                .iter()
                .map(|entry| entry.display_name.clone())
                .collect();
            let planned = plan_batch_rename(&names, &planned_mode, &batch_rename_timestamp());
            let items: Vec<_> = entries.iter().cloned().zip(planned).collect();
            if state.browser.rename_many(items).is_none() {
                show_error_dialog(
                    &overlay_for_error,
                    "Nothing to rename",
                    "Every name is unchanged, invalid, or duplicated.",
                );
                return;
            }
            dismiss_for_confirm();
        });
        find_entry.grab_focus();
    }
}

fn batch_rename_section() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 6)
}

/// Raw dialog input for one preview or confirm pass.
struct RenameDialogInput<'a> {
    mode: RenameDialogMode,
    find: &'a str,
    replace_with: &'a str,
    text: &'a str,
    before_name: bool,
    base: &'a str,
    style: FormatStyle,
    start: &'a str,
    format_before: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RenameDialogError {
    MissingFind,
    MissingText,
    MissingBase,
    BadStart,
}

fn dialog_rename_mode(input: &RenameDialogInput) -> Result<BatchRenameMode, RenameDialogError> {
    match input.mode {
        RenameDialogMode::Replace => {
            if input.find.is_empty() {
                return Err(RenameDialogError::MissingFind);
            }
            Ok(BatchRenameMode::ReplaceText {
                find: input.find.to_owned(),
                replace_with: input.replace_with.to_owned(),
            })
        }
        RenameDialogMode::Add => {
            if input.text.is_empty() {
                return Err(RenameDialogError::MissingText);
            }
            Ok(BatchRenameMode::AddText {
                text: input.text.to_owned(),
                before_name: input.before_name,
            })
        }
        RenameDialogMode::Format => {
            if input.base.is_empty() {
                return Err(RenameDialogError::MissingBase);
            }
            let Ok(start_number) = input.start.parse::<usize>() else {
                return Err(RenameDialogError::BadStart);
            };
            Ok(BatchRenameMode::Format {
                custom_name: input.base.to_owned(),
                style: input.style,
                start_number: start_number.max(1),
                before_name: input.format_before,
            })
        }
    }
}

/// Previews the first entry's planned name for the example line.
fn plan_preview_name(entries: &[FileEntry], input: &RenameDialogInput) -> Option<(String, String)> {
    let first = entries.first()?;
    let planned_mode = dialog_rename_mode(input).ok()?;
    let planned = plan_batch_rename(
        std::slice::from_ref(&first.display_name),
        &planned_mode,
        &batch_rename_timestamp(),
    );
    Some((first.display_name.clone(), planned.into_iter().next()?))
}

fn batch_rename_timestamp() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|moment| moment.format("%Y-%m-%d at %I.%M.%S %p").ok())
        .map(|moment| moment.to_string())
        .unwrap_or_default()
}

fn flag_required(field: &gtk::Entry, message: &str) {
    field.add_css_class("error");
    field.set_tooltip_text(Some(message));
    field.grab_focus();
}
