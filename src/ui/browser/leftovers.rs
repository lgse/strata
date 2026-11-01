// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::OsString,
    rc::Rc,
    time::Duration,
};

use gtk::prelude::*;

use crate::{
    app::Browser,
    model::{FileEntry, Location, hidden_names::Leftover},
    services::{LeftoverInside, LoadHandle, ReviewedLeftover},
    ui::{
        accessibility,
        browser::{ViewState, entry_icon},
        controls::{ModalTone, focus_button, message_dialog_description, message_dialog_layout},
        modal::{ModalHost, close_on_escape, dismiss_modal_layer_then, modal_layer},
        progress_dock::{CompactProgress, Notice},
    },
};

#[cfg(test)]
mod tests;

const CARD_SHOWN_FOR: Duration = Duration::from_secs(15);

type AfterClose = Option<Box<dyn FnOnce()>>;

#[derive(Default)]
pub(super) struct LeftoverState {
    review: RefCell<Option<LoadHandle>>,
    recovered: RefCell<Option<Location>>,
}

#[derive(Clone, Copy)]
enum Action {
    Restore,
    Show,
    Delete,
}

struct Row {
    date: gtk::Label,
    contains: gtk::Label,
    inside: gtk::Label,
    restore: Option<gtk::Button>,
}

/// What the row's "Contains" line says, or a status line without that label.
enum Inside {
    Contains(String),
    Status(String),
}

fn title() -> String {
    crate::i18n::tr("Items left by an interrupted operation")
}

fn heading(kind: Leftover) -> String {
    crate::i18n::tr(match kind {
        Leftover::Originals => "Set-aside originals",
        Leftover::MergeOriginals => "Originals set aside by a merge",
        Leftover::Original => "Set-aside original",
        Leftover::MaybeOriginal => "Possible original",
        Leftover::IncompleteCopy | Leftover::Probe => "Incomplete copy",
    })
}

fn explanation(kind: Leftover) -> String {
    crate::i18n::tr(match kind {
        Leftover::Originals => {
            "Originals moved aside before being replaced. Restore puts them back."
        }
        Leftover::MergeOriginals => {
            "Originals a merge moved aside before replacing them. Restore puts them back."
        }
        Leftover::Original => {
            "An original an earlier version kept under a hidden name. Restore brings it back under a new name."
        }
        Leftover::MaybeOriginal => {
            "May be a replaced original or an incomplete copy. Restore brings it back under a new name."
        }
        Leftover::IncompleteCopy | Leftover::Probe => {
            "A copy that never finished. Its source was not changed."
        }
    })
}

fn restorable(kind: Leftover, folder: &Location) -> bool {
    match kind {
        Leftover::Originals | Leftover::MergeOriginals => folder.native_path().is_some(),
        Leftover::Original | Leftover::MaybeOriginal => true,
        Leftover::IncompleteCopy | Leftover::Probe => false,
    }
}

fn recovered_name(taken: &[String]) -> String {
    let base = crate::i18n::tr("Recovered item");
    std::iter::once(base.clone())
        .chain((1..).map(|number| format!("{base} ({number})")))
        .find(|name| !taken.contains(name))
        .unwrap_or(base)
}

fn describe(inside: &LeftoverInside) -> Option<Inside> {
    match inside {
        LeftoverInside::Item => None,
        LeftoverInside::Items { names, more } if names.is_empty() && *more == 0 => {
            Some(Inside::Contains(crate::i18n::tr("Nothing inside")))
        }
        LeftoverInside::Items { names, more } => {
            let mut lines: Vec<String> = names
                .iter()
                .map(|name| name.to_string_lossy().into_owned())
                .collect();
            if *more > 0 {
                lines.push(crate::i18n::count("more_items", *more));
            }
            Some(Inside::Contains(lines.join("\n")))
        }
        LeftoverInside::StillOpen(reason) | LeftoverInside::Unreadable(reason) => {
            Some(Inside::Status(reason.clone()))
        }
        LeftoverInside::Gone => Some(Inside::Status(crate::i18n::tr("No longer here"))),
    }
}

fn card_label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
    label.add_css_class(class);
    label
}

/// A grid cell: wrapping labels make a grid request their narrowest layout's height.
fn detail_label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    label.add_css_class(class);
    label
}

impl ViewState {
    pub(super) fn follow_leftovers(self: &Rc<Self>, depth: usize, announce: bool) {
        let shown = !self.browser.column_leftovers(depth).is_empty();
        if let Some(column) = self.columns.borrow().get(depth) {
            column.leftovers_button.set_visible(shown);
        }
        self.mode_views.borrow().show_leftovers(depth, shown);
        if announce && shown && self.interactive {
            self.show_leftovers_card(depth);
        }
    }

    pub(super) fn leftover_kind(&self, depth: usize, entry: &FileEntry) -> Option<Leftover> {
        self.browser
            .column_leftovers(depth)
            .into_iter()
            .find(|(leftover, _)| leftover.location == entry.location)
            .map(|(_, kind)| kind)
            .filter(|kind| {
                self.browser
                    .location_at(depth)
                    .is_some_and(|folder| restorable(*kind, &folder))
            })
    }

    pub(super) fn take_recovered_item(&self) -> Option<Location> {
        self.leftovers.recovered.take()
    }

    fn refused_while_busy(&self, folder: &Location) -> bool {
        let busy = Browser::leftovers_busy(folder);
        if busy {
            crate::ui::modal::show_error_dialog(
                &self.overlay,
                &title(),
                &crate::i18n::tr("Wait for the running operation to finish, then try again."),
            );
        }
        busy
    }

    fn show_leftovers_card(self: &Rc<Self>, depth: usize) {
        let Some(folder) = self.browser.location_at(depth) else {
            return;
        };
        let overlay =
            crate::ui::modal::window_overlay(&self.overlay).unwrap_or_else(|| self.overlay.clone());
        let card = CompactProgress::notice(
            &overlay,
            crate::assets::icons::TRIANGLE_ALERT,
            Notice {
                title: &title(),
                text: &rust_i18n::t!(
                    "“%{folder}” holds originals that an interrupted operation set aside.",
                    folder = folder.display_name()
                ),
                action: &crate::i18n::tr("Review"),
                shown_for: CARD_SHOWN_FOR,
            },
        );
        let weak = Rc::downgrade(self);
        card.complete.connect_clicked(move |_| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            match state.browser.open_depth(&folder) {
                Some(depth) => state.show_leftover_review(depth),
                None => state.browser.navigate(folder.clone()),
            }
        });
    }

    pub(super) fn show_leftover_review(self: &Rc<Self>, depth: usize) {
        let leftovers = self.browser.column_leftovers(depth);
        let Some(folder) = self.browser.location_at(depth) else {
            return;
        };
        if leftovers.is_empty() {
            self.refused_while_busy(&folder);
            return;
        }
        let Some(ModalHost {
            overlay: window_overlay,
            blurred_root,
        }) = ModalHost::blurred_for(&self.overlay)
        else {
            return;
        };
        let layout = message_dialog_layout(
            crate::assets::icons::TRIANGLE_ALERT,
            &title(),
            &rust_i18n::t!("In “%{folder}”", folder = folder.display_name()),
            &crate::i18n::tr("Close"),
            ModalTone::Accent,
        );
        layout.cancel.set_visible(false);
        layout.set_loading(
            true,
            Some(&crate::i18n::tr("Reading what each item holds…")),
        );
        let layer = modal_layer(&layout.content, &window_overlay, blurred_root.clone(), None);
        let closed = Rc::new(Cell::new(false));
        let dismiss: Rc<dyn Fn(AfterClose)> =
            {
                let (layer, overlay, closed) =
                    (layer.clone(), window_overlay.clone(), closed.clone());
                let weak = Rc::downgrade(self);
                Rc::new(move |then: AfterClose| {
                    if closed.replace(true) {
                        return;
                    }
                    let weak = weak.clone();
                    if let Some(state) = weak.upgrade() {
                        state.leftovers.review.take();
                    }
                    dismiss_modal_layer_then(&layer, &overlay, blurred_root.as_ref(), move || {
                        match then {
                            Some(then) => then(),
                            None => {
                                if let Some(state) = weak.upgrade() {
                                    state.browser.focus_active();
                                }
                            }
                        }
                    });
                })
            };

        let list = gtk::Box::new(gtk::Orientation::Vertical, 10);
        list.add_css_class("delete-confirmation-files");
        let mut rows = HashMap::new();
        for (entry, kind) in &leftovers {
            let card = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            card.add_css_class("leftover-card");
            let icon = crate::assets::primary_icon(entry_icon(entry), 16);
            icon.set_valign(gtk::Align::Start);
            card.append(&icon);
            let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
            body.set_hexpand(true);
            body.append(&card_label(&heading(*kind), "leftover-card-title"));
            let date = gtk::Label::new(None);
            date.set_xalign(0.0);
            date.add_css_class("leftover-card-detail");
            body.append(&date);
            let about = card_label(&explanation(*kind), "leftover-card-text");
            about.set_margin_top(6);
            body.append(&about);
            let details = gtk::Grid::builder()
                .row_spacing(2)
                .column_spacing(10)
                .margin_top(6)
                .build();
            let contains = detail_label(&crate::i18n::tr("Contains"), "leftover-card-label");
            contains.set_valign(gtk::Align::Start);
            contains.set_visible(false);
            let inside = detail_label("", "leftover-card-value");
            inside.set_visible(false);
            details.attach(&contains, 0, 0, 1, 1);
            details.attach(&inside, 1, 0, 1, 1);
            details.attach(
                &detail_label(&crate::i18n::tr("On disk"), "leftover-card-label"),
                0,
                1,
                1,
                1,
            );
            let name = gtk::Label::builder()
                .label(&entry.display_name)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .selectable(true)
                .build();
            name.add_css_class("leftover-card-value");
            details.attach(&name, 1, 1, 1, 1);
            body.append(&details);
            card.append(&body);
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            actions.set_halign(gtk::Align::End);
            actions.set_valign(gtk::Align::Center);
            let mut restore = None;
            for (action, label, offered) in [
                (Action::Restore, "Restore", restorable(*kind, &folder)),
                (Action::Show, "Show", true),
                (Action::Delete, "Delete…", *kind == Leftover::IncompleteCopy),
            ] {
                if !offered {
                    continue;
                }
                let button = gtk::Button::with_label(&crate::i18n::tr(label));
                button.add_css_class("update-dialog-action");
                let (weak, dismiss, entry, kind) =
                    (Rc::downgrade(self), dismiss.clone(), entry.clone(), *kind);
                button.connect_clicked(move |_| {
                    let weak = weak.clone();
                    let entry = entry.clone();
                    dismiss(Some(Box::new(move || {
                        if let Some(state) = weak.upgrade() {
                            state.act_on_leftover(depth, entry, kind, action);
                        }
                    })));
                });
                if matches!(action, Action::Restore) {
                    restore = Some(button.clone());
                }
                actions.append(&button);
            }
            card.append(&actions);
            list.append(&card);
            rows.insert(
                entry.native_name.clone(),
                Row {
                    date,
                    contains,
                    inside,
                    restore,
                },
            );
        }
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .max_content_height(400)
            .propagate_natural_height(true)
            .build();
        scroller.add_css_class("delete-confirmation-list");
        layout.body.append(&scroller);
        layout.body.append(&message_dialog_description(&crate::i18n::tr(
            "Another program or computer may still be using these items. Restore never replaces an existing item.",
        )));
        window_overlay.add_overlay(&layer);
        let closing = dismiss.clone();
        layout.close.connect_clicked(move |_| closing(None));
        let closing = dismiss.clone();
        layout.confirm.connect_clicked(move |_| closing(None));
        close_on_escape(&layer, move || dismiss(None));
        focus_button(&layout.confirm);

        let names: Vec<OsString> = leftovers
            .iter()
            .map(|(entry, _)| entry.native_name.clone())
            .collect();
        let loading = layout.loading.downgrade();
        let answered = closed.clone();
        let review = self.browser.review_leftovers(
            &folder,
            names,
            Box::new(move |reviewed: Vec<ReviewedLeftover>| {
                if answered.get() {
                    return;
                }
                for reviewed in reviewed {
                    let Some(row) = rows.get(&reviewed.name) else {
                        continue;
                    };
                    crate::util::set_full_modified_date(&row.date, reviewed.modified);
                    match describe(&reviewed.inside) {
                        Some(Inside::Contains(text)) => {
                            row.inside.set_text(&text);
                            row.inside.set_visible(true);
                            row.contains.set_visible(true);
                        }
                        Some(Inside::Status(text)) => {
                            row.inside.set_text(&text);
                            row.inside.set_visible(true);
                        }
                        None => {}
                    }
                    if let (Some(restore), LeftoverInside::StillOpen(_) | LeftoverInside::Gone) =
                        (&row.restore, &reviewed.inside)
                    {
                        restore.set_visible(false);
                    }
                }
                if let Some(loading) = loading.upgrade() {
                    loading.stop();
                    loading.set_visible(false);
                    accessibility::set_description(&loading, None);
                }
            }),
        );
        if !closed.get() {
            self.leftovers.review.replace(Some(review));
        }
    }

    fn act_on_leftover(
        self: &Rc<Self>,
        depth: usize,
        entry: FileEntry,
        kind: Leftover,
        action: Action,
    ) {
        let Some(folder) = entry.location.parent() else {
            return;
        };
        match action {
            Action::Show => self.reveal_locations(folder, vec![entry.location], false),
            Action::Delete if !self.refused_while_busy(&folder) => {
                self.request_confirmed_delete(vec![entry], false);
            }
            Action::Delete => {}
            Action::Restore => self.restore_leftover(depth, entry, kind),
        }
    }

    pub(super) fn restore_leftover(
        self: &Rc<Self>,
        depth: usize,
        entry: FileEntry,
        kind: Leftover,
    ) {
        let Some(folder) = entry.location.parent() else {
            return;
        };
        if self.refused_while_busy(&folder) {
            return;
        }
        if matches!(kind, Leftover::Originals | Leftover::MergeOriginals) {
            self.browser.recover(entry.location);
            return;
        }
        let taken = self
            .browser
            .column_snapshot(depth)
            .and_then(|snapshot| {
                self.browser
                    .with_entries(depth, 0..snapshot.count, |entries| {
                        entries
                            .iter()
                            .map(|entry| entry.display_name.clone())
                            .collect::<Vec<_>>()
                    })
            })
            .unwrap_or_default();
        let name = recovered_name(&taken);
        let Some(recovered) = folder.child(std::ffi::OsStr::new(&name)) else {
            return;
        };
        self.expect_new_entry(depth, folder);
        self.leftovers.recovered.replace(Some(recovered));
        if self.browser.rename(entry, name).is_none() {
            self.leftovers.recovered.take();
            self.cancel_new_entry();
        }
    }
}
