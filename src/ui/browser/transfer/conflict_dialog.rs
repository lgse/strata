// SPDX-License-Identifier: MIT

use super::{ConflictFocus, DialogLayer};
use crate::ui::browser::ViewState;
use crate::ui::controls::{
    ModalLayout, ModalTone, focus_button, form_check_button, message_dialog_description,
    message_dialog_layout, stack_actions_when_constrained, wrap_button_label,
};
use crate::ui::keys::{
    self,
    bindings::{ENTER_ANY, ESCAPE_ANY},
};
use crate::ui::modal::ModalHost;
use gtk::glib;
use gtk::prelude::*;
use std::rc::Rc;

#[derive(Clone, Copy)]
pub(super) enum ConflictChoice {
    Replace,
    Merge,
    Skip,
    KeepBoth,
}

#[derive(Clone, Copy, Default)]
pub(super) struct ConflictActions {
    pub(super) replace: bool,
    pub(super) keep_both: bool,
    pub(super) merge: bool,
    pub(super) focus: ConflictFocus,
}

/// What one conflict prompt shows besides Cancel.
pub(super) struct ConflictPrompt<'a> {
    pub(super) name: &'a str,
    pub(super) explanation: &'a str,
    pub(super) apply_to_all_visible: bool,
    pub(super) skip_visible: bool,
    pub(super) actions: ConflictActions,
}

impl ConflictPrompt<'_> {
    /// Apply to All only matters with collisions left; Skip only when something else remains.
    pub(super) fn visibility(remaining: usize, accepted: usize) -> (bool, bool) {
        (remaining > 0, accepted > 0 || remaining > 0)
    }
}

struct ConflictDialog {
    layout: ModalLayout,
    apply_all: gtk::CheckButton,
    skip: gtk::Button,
    keep_both: gtk::Button,
    merge: gtk::Button,
}

fn secondary_action(label: &str, visible: bool) -> gtk::Button {
    let button = gtk::Button::with_label(&crate::i18n::tr(label));
    button.add_css_class("action-dialog-cancel");
    button.set_visible(visible);
    button
}

impl ConflictDialog {
    fn build(prompt: &ConflictPrompt) -> Self {
        let layout = message_dialog_layout(
            crate::assets::icons::COPY,
            &crate::i18n::tr("File already exists"),
            prompt.name,
            &crate::i18n::tr("Replace"),
            ModalTone::Danger,
        );
        layout
            .body
            .append(&message_dialog_description(prompt.explanation));
        let apply_all = form_check_button(&crate::i18n::tr("Apply to All"));
        apply_all.set_visible(prompt.apply_to_all_visible);
        layout.body.append(&apply_all);
        let skip = secondary_action("Skip", prompt.skip_visible);
        layout
            .actions
            .insert_child_after(&skip, Some(&layout.cancel));
        let keep_both = secondary_action("Keep Both", prompt.actions.keep_both);
        layout.actions.insert_child_after(&keep_both, Some(&skip));
        let merge = secondary_action("Merge", prompt.actions.merge);
        layout.actions.insert_child_after(&merge, Some(&keep_both));
        layout.confirm.set_visible(prompt.actions.replace);
        for button in [&layout.cancel, &skip, &keep_both, &merge, &layout.confirm] {
            wrap_button_label(button);
        }
        stack_actions_when_constrained(&layout);
        Self {
            layout,
            apply_all,
            skip,
            keep_both,
            merge,
        }
    }

    fn connect_choices(&self, modal: &DialogLayer, on_choice: &Rc<dyn Fn(ConflictChoice, bool)>) {
        for (button, choice) in [
            (&self.skip, ConflictChoice::Skip),
            (&self.keep_both, ConflictChoice::KeepBoth),
            (&self.merge, ConflictChoice::Merge),
            (&self.layout.confirm, ConflictChoice::Replace),
        ] {
            let modal = modal.clone();
            let apply_all = self.apply_all.clone();
            let chosen = on_choice.clone();
            button.connect_clicked(move |_| {
                modal.dismiss();
                chosen(choice, apply_all.is_active());
            });
        }
    }

    /// A prompt without Replace starts on Keep Both, or on Cancel, which never changes history,
    /// when it offers neither.
    fn initial_focus(&self, actions: ConflictActions) -> &gtk::Button {
        let keep_both_first = actions.focus == ConflictFocus::KeepBoth || !actions.replace;
        if keep_both_first && actions.keep_both {
            &self.keep_both
        } else if actions.replace {
            &self.layout.confirm
        } else {
            &self.layout.cancel
        }
    }

    fn install_keys(&self, modal: &DialogLayer) {
        let escape = gtk::EventControllerKey::new();
        escape.set_propagation_phase(gtk::PropagationPhase::Capture);
        let escaped = modal.clone();
        let enter_buttons = [
            self.skip.clone(),
            self.keep_both.clone(),
            self.merge.clone(),
            self.layout.confirm.clone(),
            self.layout.cancel.clone(),
            self.layout.close.clone(),
        ];
        keys::on_press(&escape, move |_, press| {
            if press.is(ESCAPE_ANY) {
                escaped.dismiss();
                glib::Propagation::Stop
            } else if press.is(ENTER_ANY) {
                activate_focused_button(&enter_buttons)
            } else {
                glib::Propagation::Proceed
            }
        });
        modal.layer.add_controller(escape);
    }
}

fn activate_focused_button(buttons: &[gtk::Button]) -> glib::Propagation {
    let Some(button) = buttons.iter().find(|button| button.has_focus()) else {
        return glib::Propagation::Proceed;
    };
    button.emit_clicked();
    glib::Propagation::Stop
}

impl ViewState {
    /// Cancelling abandons the whole operation without calling `on_choice`.
    pub(super) fn confirm_replace_conflict(
        self: &Rc<Self>,
        prompt: ConflictPrompt,
        on_choice: Rc<dyn Fn(ConflictChoice, bool)>,
    ) {
        let Some(host) = ModalHost::blurred_for(&self.overlay) else {
            return;
        };
        let dialog = ConflictDialog::build(&prompt);
        let modal = DialogLayer::new(&dialog.layout.content, host);
        modal.show();
        let browser = Rc::downgrade(&self.browser);
        modal.layer.connect_parent_notify(move |layer| {
            if layer.parent().is_none()
                && let Some(browser) = browser.upgrade()
            {
                browser.focus_active();
            }
        });
        modal.dismiss_on_click(&dialog.layout.cancel);
        modal.dismiss_on_click(&dialog.layout.close);
        dialog.connect_choices(&modal, &on_choice);
        dialog.install_keys(&modal);
        focus_button(dialog.initial_focus(prompt.actions));
    }
}
