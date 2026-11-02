// SPDX-License-Identifier: MIT

use super::TrashModal;
use crate::adapters::directory_summary::{DirectorySummary, summarize_directory};
use crate::adapters::trash::{EmptyTrashOutcome, empty_trash};
use crate::app::Browser;
use crate::model::Location;
use crate::services::LoadHandle;
use crate::ui::browser::ViewState;
use crate::ui::browser::entry::{format_file_size, item_count_label};
use crate::ui::controls::{
    ModalTone, focus_button, message_dialog_description, message_dialog_layout, modal_layout,
};
use crate::ui::modal::{ModalHost, close_on_escape, modal_layer, show_error_dialog};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

fn empty_trash_error_summary(outcome: &EmptyTrashOutcome) -> String {
    let mut summary = rust_i18n::t!(
        "%{items} could not be deleted. The remaining items were processed.",
        items = item_count_label(outcome.failed)
    )
    .into_owned();
    for error in &outcome.errors {
        summary.push_str("\n\n• ");
        summary.push_str(error);
    }
    if outcome.failed > outcome.errors.len() {
        summary.push_str("\n\n");
        summary.push_str(&crate::i18n::count(
            "more_items",
            outcome.failed - outcome.errors.len(),
        ));
    }
    summary
}

fn log_trash_summary(summary: &DirectorySummary, started: Instant) {
    let elapsed_ms = started.elapsed().as_millis() as u64;
    if summary.truncated() {
        tracing::warn!(
            item_count = summary.item_count,
            elapsed_ms = elapsed_ms,
            "trash summary truncated"
        );
    } else {
        tracing::info!(
            item_count = summary.item_count,
            elapsed_ms = elapsed_ms,
            "trash summary built"
        );
    }
}

fn empty_trash_subtitle(summary: &DirectorySummary) -> String {
    rust_i18n::t!(
        if summary.truncated() {
            "At least %{items} · at least %{size} will be reclaimed"
        } else {
            "%{items} · %{size} will be reclaimed"
        },
        items = item_count_label(summary.item_count),
        size = format_file_size(summary.total_size)
    )
    .into_owned()
}

fn report_empty_trash_result(
    result: Result<EmptyTrashOutcome, glib::Error>,
    browser: &Rc<Browser>,
    error_overlay: &gtk::Overlay,
) {
    match result {
        Ok(outcome) => {
            // A wholesale refresh rather than tracking each deleted location, to
            // keep this operation's own state bounded too.
            browser.refresh_columns_at(&Location::uri("trash:///"));
            if outcome.failed > 0 {
                crate::ui::modal::show_partial_failure_dialog(
                    error_overlay,
                    &empty_trash_error_summary(&outcome),
                );
            }
        }
        Err(error) => {
            tracing::warn!(
                error_domain = ?error.domain(),
                error_code = error.code(),
                "empty trash failed"
            );
            show_error_dialog(
                error_overlay,
                &crate::i18n::tr("Unable to empty Trash"),
                &error.to_string(),
            );
        }
    }
}

impl ViewState {
    /// Safe to call more than once: whichever of cancel or completion runs first leaves the
    /// other a no-op.
    fn clear_empty_trash(self: &Rc<Self>) {
        self.pending_empty_trash.borrow_mut().take();
        self.dismiss_file_operation_progress();
    }

    fn trash_listing_is_empty(&self) -> bool {
        self.columns
            .borrow()
            .iter()
            .find(|column| column.empty_trash_button.is_some())
            .is_some_and(|column| column.entry_count.get() == 0)
    }

    pub(in crate::ui::browser) fn load_trash_summary(self: &Rc<Self>) {
        if self.trash_listing_is_empty() {
            return;
        }
        if !self.show_trash_loading_indicator(
            "Measuring Trash…",
            "Empty Trash",
            "Calculating the number and size of items. This may take a few seconds.",
        ) {
            return;
        }
        let weak = Rc::downgrade(self);
        let started = Instant::now();
        let task = glib::MainContext::default().spawn_local(async move {
            // Let GTK paint the loading dialog before beginning a walk whose GIO futures may be
            // immediately ready for long stretches on a fast local trash backend.
            glib::timeout_future(Duration::from_millis(16)).await;
            let trash = gio::File::for_uri("trash:///");
            let result = summarize_directory(&trash).await;
            finish_trash_summary(&weak, result, started);
        });
        self.pending_trash_lookup
            .replace(Some(LoadHandle::new(move || {
                tracing::debug!("trash summary cancelled");
                task.abort();
            })));
    }

    /// The walk is bounded but can still take a few seconds on a large trash, hence the indicator.
    pub(super) fn show_trash_loading_indicator(
        self: &Rc<Self>,
        title: &str,
        action: &str,
        detail: &str,
    ) -> bool {
        self.clear_trash_loading();
        let Some(host) = ModalHost::blurred_for(&self.overlay) else {
            show_error_dialog(
                &self.overlay,
                &crate::i18n::tr("Unable to continue"),
                &crate::i18n::tr("The operation could not be confirmed."),
            );
            return false;
        };

        let title = crate::i18n::tr(title);
        let layout = modal_layout(
            crate::assets::icons::TRASH,
            &title,
            "",
            &crate::i18n::tr(action),
        );
        layout.set_loading(true, Some(&title));
        layout.subtitle.set_visible(false);
        layout.confirm.set_visible(false);
        let explanation = message_dialog_description(&crate::i18n::tr(detail));
        layout.body.append(&explanation);
        let cancel = layout.cancel;
        let close = layout.close;

        let layer = modal_layer(
            &layout.content,
            &host.overlay,
            host.blurred_root.clone(),
            Some(Rc::new(|| true)),
        );
        host.overlay.add_overlay(&layer);
        self.trash_loading.replace(Some(TrashModal {
            layer,
            overlay: host.overlay,
            blurred_root: host.blurred_root,
        }));

        for button in [&cancel, &close] {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.clear_trash_loading();
                }
            });
        }
        let weak_escape = Rc::downgrade(self);
        if let Some(view) = self.trash_loading.borrow().as_ref() {
            close_on_escape(&view.layer, move || {
                if let Some(state) = weak_escape.upgrade() {
                    state.clear_trash_loading();
                }
            });
        }
        cancel.grab_focus();
        true
    }

    fn dismiss_trash_loading(&self) {
        let Some(view) = self.trash_loading.take() else {
            return;
        };
        view.dismiss();
    }

    /// Safe to call more than once: whichever of cancel or completion runs first leaves the
    /// other a no-op.
    pub(super) fn clear_trash_loading(&self) {
        self.pending_trash_lookup.borrow_mut().take();
        self.dismiss_trash_loading();
    }

    fn show_empty_trash_confirmation(self: &Rc<Self>, summary: DirectorySummary) {
        let Some(host) = ModalHost::blurred_for(&self.overlay) else {
            return;
        };

        let layout = message_dialog_layout(
            crate::assets::icons::TRASH,
            &crate::i18n::tr("Empty Trash?"),
            &empty_trash_subtitle(&summary),
            &crate::i18n::tr("Empty Trash"),
            ModalTone::Danger,
        );
        let explanation = message_dialog_description(&crate::i18n::tr(
            "Everything in Trash will be permanently deleted. This action cannot be undone.",
        ));
        layout.body.append(&explanation);
        let empty = layout.confirm;

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
        let empty_modal = modal.clone();
        let browser = self.browser.clone();
        let error_overlay = self.overlay.clone();
        let weak_ui = Rc::downgrade(self);
        empty.connect_clicked(move |_| {
            empty_modal.dismiss();
            browser.focus_active();
            start_empty_trash(&weak_ui, &browser, &error_overlay);
        });
        modal.install_focus_keys(&self.browser, false);
        focus_button(&empty);
    }
}

fn finish_trash_summary(
    weak: &Weak<ViewState>,
    result: Result<DirectorySummary, glib::Error>,
    started: Instant,
) {
    match result {
        Ok(summary) if summary.item_count > 0 => {
            log_trash_summary(&summary, started);
            if let Some(state) = weak.upgrade() {
                state.clear_trash_loading();
                state.show_empty_trash_confirmation(summary);
            }
        }
        Ok(_) => {
            if let Some(state) = weak.upgrade() {
                state.clear_trash_loading();
            }
        }
        Err(error) => {
            tracing::warn!(
                error_domain = ?error.domain(),
                error_code = error.code(),
                "trash summary failed"
            );
            if let Some(state) = weak.upgrade() {
                state.clear_trash_loading();
                show_error_dialog(
                    &state.overlay,
                    &crate::i18n::tr("Unable to read Trash"),
                    &error.to_string(),
                );
            }
        }
    }
}

fn start_empty_trash(
    weak_ui: &Weak<ViewState>,
    browser: &Rc<Browser>,
    error_overlay: &gtk::Overlay,
) {
    let cancel_ui = weak_ui.clone();
    let on_cancel: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(ui) = cancel_ui.upgrade() {
            ui.clear_empty_trash();
        }
    });
    if let Some(ui) = weak_ui.upgrade() {
        ui.show_empty_trash_progress(on_cancel);
    }

    let error_overlay = error_overlay.clone();
    let progress_ui = weak_ui.clone();
    let finish_ui = weak_ui.clone();
    let finish_browser = browser.clone();
    let task = glib::MainContext::default().spawn_local(async move {
        let trash = gio::File::for_uri("trash:///");
        let result = empty_trash(&trash, move |processed| {
            if let Some(ui) = progress_ui.upgrade() {
                ui.update_empty_trash_progress(processed);
            }
        })
        .await;
        if let Some(ui) = finish_ui.upgrade() {
            ui.clear_empty_trash();
        }
        report_empty_trash_result(result, &finish_browser, &error_overlay);
    });
    if let Some(ui) = weak_ui.upgrade() {
        ui.pending_empty_trash
            .replace(Some(LoadHandle::new(move || {
                tracing::debug!("empty trash cancelled");
                task.abort();
            })));
    }
}
