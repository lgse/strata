// SPDX-License-Identifier: MIT

use std::rc::Rc;

use gtk::glib;

use crate::{
    model::FileEntry,
    services::DeleteBlocker,
    ui::{
        browser::ViewState,
        modal::{
            DeleteErrorAction, show_delete_error_dialog, show_error_dialog,
            show_partial_failure_dialog,
        },
    },
};

pub(in crate::ui::browser) struct DeleteErrorRetry {
    pub entries: Vec<FileEntry>,
    pub require_choice: bool,
    pub run: fn(&Rc<ViewState>, Vec<FileEntry>),
}

impl ViewState {
    pub(in crate::ui::browser) fn show_partial_operation_failure(
        self: &Rc<Self>,
        message: &str,
        blocker: Option<DeleteBlocker>,
        retry: DeleteErrorRetry,
    ) {
        if !retry.entries.is_empty() && !retry.require_choice {
            (retry.run)(self, retry.entries);
            return;
        }
        let reveal = blocker.map(|blocker| self.delete_reveal_action(blocker));
        let retry = (!retry.entries.is_empty()).then(|| self.delete_retry_action(retry));
        if reveal.is_none() && retry.is_none() {
            show_partial_failure_dialog(&self.overlay, message);
        } else {
            show_delete_error_dialog(&self.overlay, message, retry, reveal);
        }
    }

    fn delete_retry_action(self: &Rc<Self>, retry: DeleteErrorRetry) -> DeleteErrorAction {
        let weak = Rc::downgrade(self);
        DeleteErrorAction {
            label: super::delete_retry_label(&retry.entries),
            run: Rc::new(move || {
                if let Some(state) = weak.upgrade() {
                    (retry.run)(&state, retry.entries.clone());
                }
            }),
        }
    }

    fn delete_reveal_action(self: &Rc<Self>, blocker: DeleteBlocker) -> DeleteErrorAction {
        let weak = Rc::downgrade(self);
        DeleteErrorAction {
            label: reveal_label(&blocker),
            run: Rc::new(move || {
                if let Some(state) = weak.upgrade() {
                    state.reveal_delete_blocker(blocker.clone());
                }
            }),
        }
    }

    fn reveal_delete_blocker(self: &Rc<Self>, blocker: DeleteBlocker) {
        let weak = Rc::downgrade(self);
        let generation = self.browser.navigation_generation();
        glib::MainContext::default().spawn_local(async move {
            let target = blocker.reveal_target().await;
            let Some(state) = weak
                .upgrade()
                .filter(|state| state.browser.navigation_generation() == generation)
            else {
                return;
            };
            match target {
                Some((folder, item)) => state.reveal_locations(folder, vec![item], false),
                None => show_error_dialog(
                    &state.overlay,
                    &crate::i18n::tr("Unable to open location"),
                    &crate::i18n::tr("The item's location could not be verified."),
                ),
            }
        });
    }
}

fn reveal_label(blocker: &DeleteBlocker) -> String {
    match blocker.child.file_name() {
        Some(name) if blocker.one_of_several => {
            rust_i18n::t!("Show “%{name}” in Folder", name = name.to_string_lossy()).into_owned()
        }
        _ => crate::i18n::tr("Show in Folder"),
    }
}

#[cfg(test)]
mod tests;
