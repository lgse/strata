// SPDX-License-Identifier: MIT

use std::{cell::Cell, rc::Rc, time::Duration};

use gtk::{glib, prelude::*};

use super::PreviewUnit;
use crate::services::{DocumentUnit, DocumentUnitKind};

pub(super) fn code_copy_button(units: Rc<Vec<PreviewUnit>>, index: usize) -> gtk::Button {
    let button = gtk::Button::builder()
        .tooltip_text(crate::i18n::tr("Copy code"))
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .build();
    button.add_css_class("preview-code-copy");
    button.set_has_frame(false);
    button.set_cursor_from_name(Some("pointer"));
    button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr(
        "Copy code",
    ))]);
    let icon = crate::assets::primary_icon(crate::assets::icons::COPY, 15);
    button.set_child(Some(&icon));

    let feedback_generation = Rc::new(Cell::new(0_u64));
    button.connect_clicked(move |button| {
        let Some(text) = code_block_copy_text(&units, index) else {
            return;
        };
        button.clipboard().set_text(&text);
        let generation = feedback_generation.get().wrapping_add(1);
        feedback_generation.set(generation);
        crate::assets::set_primary_icon(&icon, crate::assets::icons::CHECK);
        button.set_tooltip_text(Some(&crate::i18n::tr("Code copied")));
        button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr(
            "Code copied",
        ))]);

        let weak_button = button.downgrade();
        let weak_icon = icon.downgrade();
        let feedback_generation = feedback_generation.clone();
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            if feedback_generation.get() != generation {
                return;
            }
            if let Some(icon) = weak_icon.upgrade() {
                crate::assets::set_primary_icon(&icon, crate::assets::icons::COPY);
            }
            if let Some(button) = weak_button.upgrade() {
                button.set_tooltip_text(Some(&crate::i18n::tr("Copy code")));
                button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr(
                    "Copy code",
                ))]);
            }
        });
    });
    button
}

pub(super) fn code_block_copy_text(units: &[PreviewUnit], index: usize) -> Option<String> {
    let code = |index| match units.get(index) {
        Some(PreviewUnit::Document(
            unit @ DocumentUnit {
                kind: DocumentUnitKind::Code { list_depth, .. },
                ..
            },
        )) => Some((unit, *list_depth)),
        _ => None,
    };
    let (_, depth) = code(index)?;
    let mut start = index;
    while !code(start)?.0.first {
        start = start.checked_sub(1)?;
        if code(start)?.1 != depth {
            return None;
        }
    }

    let mut text = String::new();
    for index in start..units.len() {
        let (unit, unit_depth) = code(index)?;
        if unit_depth != depth {
            return None;
        }
        text.push_str(&unit.copy_text);
        if unit.last {
            return Some(text);
        }
    }
    None
}
