// SPDX-License-Identifier: MIT

use super::{ConflictFocus, DialogLayer, TransferRequest};
use crate::model::Location;
use crate::services::{DropCommit, VolumeRelation, transferable_drop_sources};
use crate::ui::browser::ViewState;
use crate::ui::browser::entry::item_count_label;
use crate::ui::browser::paths::compact_display_path;
use crate::ui::controls::{ModalTone, message_dialog_description, message_dialog_layout};
use crate::ui::modal::{ModalHost, remember_modal_focus_for_listing, show_error_dialog};
use gtk::prelude::*;
use std::rc::Rc;

pub(super) fn cross_volume_drop_description(volume: VolumeRelation) -> &'static str {
    match volume {
        VolumeRelation::Different => "The destination is on a different device.",
        VolumeRelation::Same | VolumeRelation::Unknown => {
            "Strata could not determine whether the destination is on the same device."
        }
    }
}

impl ViewState {
    pub(in crate::ui::browser) fn commit_file_drop(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        commit: DropCommit,
    ) {
        self.stop_drag_autoscroll();
        self.drop_active_depths.set(None);
        self.horizontal_scroll_generation
            .set(self.horizontal_scroll_generation.get().saturating_add(1));
        let sources = transferable_drop_sources(&destination, &sources);
        if sources.is_empty() {
            self.suppress_scroll_after_drop.set(false);
            return;
        }
        match commit {
            DropCommit::Copy => self.start_drop_transfer(destination, sources, false),
            DropCommit::Move => self.start_drop_transfer(destination, sources, true),
            DropCommit::Ask { volume, .. } => {
                self.confirm_cross_volume_drop(destination, sources, volume);
            }
            DropCommit::Forbidden => {
                self.suppress_scroll_after_drop.set(false);
            }
        }
    }

    fn confirm_cross_volume_drop(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        volume: VolumeRelation,
    ) {
        let Some(host) = ModalHost::blurred_for(&self.overlay) else {
            show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Unable to transfer"),
                &crate::i18n::tr("The transfer could not be confirmed."),
            );
            return;
        };

        let layout = message_dialog_layout(
            crate::assets::icons::COPY,
            &crate::i18n::tr("Copy or move?"),
            &rust_i18n::t!(
                "%{items} to %{destination}",
                items = item_count_label(sources.len()),
                destination = compact_display_path(&destination)
            ),
            &crate::i18n::tr("Copy"),
            ModalTone::Accent,
        );
        layout
            .body
            .append(&message_dialog_description(&crate::i18n::tr(
                cross_volume_drop_description(volume),
            )));
        let move_button = gtk::Button::with_label(&crate::i18n::tr("Move"));
        move_button.add_css_class("action-dialog-cancel");
        layout
            .actions
            .insert_child_after(&move_button, Some(&layout.cancel));
        let copy = layout.confirm;

        let modal = DialogLayer::new(&layout.content, host);
        remember_modal_focus_for_listing(&modal.layer, &modal.overlay);
        modal.show();
        modal.dismiss_on_click(&layout.cancel);
        modal.dismiss_on_click(&layout.close);

        for (button, move_sources) in [(&move_button, true), (&copy, false)] {
            let modal = modal.clone();
            let chosen_state = self.clone();
            let chosen_destination = destination.clone();
            let chosen_sources = sources.clone();
            button.connect_clicked(move |_| {
                modal.dismiss();
                chosen_state.start_drop_transfer(
                    chosen_destination.clone(),
                    chosen_sources.clone(),
                    move_sources,
                );
            });
        }
        modal.dismiss_on_escape();
        copy.grab_focus();
    }

    fn start_drop_transfer(
        self: &Rc<Self>,
        destination: Location,
        sources: Vec<Location>,
        move_sources: bool,
    ) {
        let reveal = crate::ui::preferences::PreferenceManager::shared().open_folder_after_drop();
        self.start_transfer_with_reveal(TransferRequest {
            destination,
            sources,
            move_sources,
            reveal,
            send_to: None,
            focus: ConflictFocus::Replace,
        });
    }
}
