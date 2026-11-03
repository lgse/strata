// SPDX-License-Identifier: MIT

use super::TrashModal;
use crate::adapters::trash_restore::restore_destinations_for_locations;
use crate::model::FileEntry;
use crate::services::{LoadHandle, RestoreTrashItem};
use crate::ui::browser::ViewState;
use crate::ui::browser::entry::{entry_icon, entry_kind_summary, item_count_label};
use crate::ui::controls::{
    ModalTone, focus_button, message_dialog_description, message_dialog_layout,
};
use crate::ui::modal::{ModalHost, show_error_dialog};
use gtk::glib;
use gtk::prelude::*;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

fn restore_confirmation_title(count: usize) -> String {
    rust_i18n::t!("Restore %{value1}?", value1 = item_count_label(count)).into_owned()
}

fn restore_confirmation_confirm_label(count: usize) -> String {
    if count == 1 {
        crate::i18n::tr("Restore")
    } else {
        rust_i18n::t!("Restore %{value1}", value1 = item_count_label(count)).into_owned()
    }
}

fn restore_destination_text(destination: &Path) -> String {
    destination.to_string_lossy().into_owned()
}

fn restore_error_summary(errors: &[String]) -> String {
    let mut summary = if errors.len() == 1 {
        errors[0].clone()
    } else {
        rust_i18n::t!(
            "%{items} could not be restored.",
            items = item_count_label(errors.len())
        )
        .into_owned()
    };
    if errors.len() > 1 {
        for error in errors.iter().take(8) {
            summary.push_str("\n\n• ");
            summary.push_str(error);
        }
        if errors.len() > 8 {
            summary.push_str("\n\n");
            summary.push_str(&crate::i18n::count("more_items", errors.len() - 8));
        }
    }
    summary
}

fn restore_explanation(count: usize, skipped: &[String]) -> String {
    let mut explanation = crate::i18n::tr(if count == 1 {
        "This item will be moved back to the original location shown above."
    } else {
        "These items will be moved back to the original locations shown above."
    });
    if !skipped.is_empty() {
        explanation.push_str("\n\n");
        explanation.push_str(&restore_error_summary(skipped));
    }
    explanation
}

fn restore_row(entry: &FileEntry, destination: &Path) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("delete-confirmation-file");
    let icon = crate::assets::primary_icon(entry_icon(entry), 16);
    let name = gtk::Label::new(Some(&entry.display_name));
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    name.set_xalign(0.0);
    let destination_label = gtk::Label::new(Some(&restore_destination_text(destination)));
    destination_label.add_css_class("delete-confirmation-file-metadata");
    destination_label.set_wrap(true);
    destination_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    destination_label.set_hexpand(true);
    destination_label.set_xalign(1.0);
    destination_label.set_selectable(true);
    crate::ui::accessibility::set_description(
        &destination_label,
        Some(&destination.to_string_lossy()),
    );
    row.append(&icon);
    row.append(&name);
    row.append(&destination_label);
    row
}

fn restore_file_list(resolved: &[(FileEntry, PathBuf)]) -> gtk::ScrolledWindow {
    let files = gtk::Box::new(gtk::Orientation::Vertical, 3);
    files.add_css_class("delete-confirmation-files");
    for (entry, destination) in resolved {
        files.append(&restore_row(entry, destination));
    }
    let file_scroller = gtk::ScrolledWindow::builder()
        .child(&files)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .max_content_height(256)
        .propagate_natural_height(true)
        .build();
    file_scroller.add_css_class("delete-confirmation-list");
    file_scroller
}

type RestoreLookups = (Vec<(FileEntry, PathBuf)>, Vec<String>);

async fn resolve_restore_destinations(entries: Vec<FileEntry>) -> RestoreLookups {
    let lookups = entries
        .iter()
        .map(|entry| (entry.location.clone(), entry.thumbnail_path.clone()))
        .collect::<Vec<_>>();
    let mut resolved = Vec::new();
    let mut errors = Vec::new();
    for (entry, destination) in entries
        .into_iter()
        .zip(restore_destinations_for_locations(lookups).await)
    {
        match destination {
            Ok(destination) => resolved.push((entry, destination)),
            Err(error) => errors.push(
                rust_i18n::t!(
                    "%{name}: %{error}",
                    name = entry.display_name.as_str(),
                    error = crate::services::error_detail(error.user_message())
                )
                .into_owned(),
            ),
        }
    }
    (resolved, errors)
}

impl ViewState {
    pub(in crate::ui::browser) fn request_restore(self: &Rc<Self>, entries: Vec<FileEntry>) {
        if entries.is_empty() {
            return;
        }
        if !self.show_trash_loading_indicator(
            "Finding restore destinations…",
            "Restore",
            "Checking the original locations before confirmation. You can cancel this lookup.",
        ) {
            return;
        }
        let weak = Rc::downgrade(self);
        let task = glib::MainContext::default().spawn_local(async move {
            glib::timeout_future(Duration::from_millis(16)).await;
            let (resolved, errors) = resolve_restore_destinations(entries).await;
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.clear_trash_loading();
            if resolved.is_empty() {
                show_error_dialog(
                    &state.overlay,
                    &crate::i18n::tr("Unable to restore"),
                    &restore_error_summary(&errors),
                );
                return;
            }
            state.show_restore_confirmation(resolved, errors);
        });
        self.pending_trash_lookup
            .replace(Some(LoadHandle::new(move || task.abort())));
    }

    fn show_restore_confirmation(
        self: &Rc<Self>,
        resolved: Vec<(FileEntry, PathBuf)>,
        skipped: Vec<String>,
    ) {
        let Some(host) = ModalHost::blurred_for(&self.overlay) else {
            show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Unable to restore"),
                &crate::i18n::tr("The restore destinations could not be confirmed."),
            );
            return;
        };

        let count = resolved.len();
        let layout = message_dialog_layout(
            crate::assets::icons::UNDO_2,
            &restore_confirmation_title(count),
            &entry_kind_summary(
                &resolved
                    .iter()
                    .map(|(entry, _)| entry.clone())
                    .collect::<Vec<_>>(),
            ),
            &restore_confirmation_confirm_label(count),
            ModalTone::Accent,
        );
        layout.body.append(&restore_file_list(&resolved));
        layout
            .body
            .append(&message_dialog_description(&restore_explanation(
                count, &skipped,
            )));
        let confirm = layout.confirm;

        let modal = TrashModal::new(&layout.content, host);
        modal.show();
        for button in [&layout.cancel, &layout.close] {
            let modal = modal.clone();
            let browser = self.browser.clone();
            button.connect_clicked(move |_| {
                modal.dismiss();
                browser.focus_active();
            });
        }
        let items = resolved
            .into_iter()
            .map(|(entry, destination)| RestoreTrashItem { entry, destination })
            .collect::<Vec<_>>();
        let confirmed_modal = modal.clone();
        let browser = self.browser.clone();
        let confirmed_state = Rc::downgrade(self);
        confirm.connect_clicked(move |_| {
            confirmed_modal.dismiss();
            if let Some(state) = confirmed_state.upgrade() {
                state.prepare_restore_flight(&items);
            }
            browser.restore(items.clone());
            browser.focus_active();
        });
        modal.install_focus_keys(&self.browser, true);
        focus_button(&confirm);
    }

    fn prepare_restore_flight(&self, items: &[RestoreTrashItem]) {
        let source = self.delete_animation_source();
        let trash_button = self.trash_button.upgrade();
        let animation = source.zip(trash_button).and_then(|(source, trash_button)| {
            crate::ui::browser::fly_to_trash::prepare_fly_from_trash(
                &source,
                items.iter().map(|item| &item.entry),
                &trash_button,
            )
        });
        self.pending_file_operation_animation.replace(animation);
    }
}
