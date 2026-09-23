// SPDX-License-Identifier: MIT

use super::collection::ResultKind;
use crate::{
    model::Location,
    services::SearchItem,
    ui::{accessibility, browser, collection_edit::EditWidgets, icons_cell, thumbnail},
};
use gtk::prelude::*;
use std::path::Path;

#[derive(Clone)]
enum Labels {
    Rows {
        name: gtk::Label,
        origin: gtk::Label,
    },
    Icons {
        name: gtk::Inscription,
        origin: gtk::Label,
    },
}

/// The factory owns its structure; behavior receives explicit editor and display handles.
#[derive(Clone)]
pub(super) struct ResultWidgets {
    pub(super) widget: gtk::Box,
    pub(super) edit: EditWidgets,
    icon: thumbnail::ThumbnailSlot,
    labels: Labels,
}

impl ResultWidgets {
    pub(super) fn new(kind: &ResultKind) -> Self {
        match kind {
            ResultKind::Rows => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.add_css_class("file-row");
                row.add_css_class("filter-result");
                let icon = thumbnail::ThumbnailSlot::new(17);
                row.append(&icon);
                let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
                labels.set_hexpand(true);
                let name = gtk::Label::builder()
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .build();
                name.add_css_class("alternate-rename-label");
                let origin = gtk::Label::builder()
                    .xalign(0.0)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .lines(2)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .build();
                origin.add_css_class("file-search-path");
                labels.append(&name);
                labels.append(&origin);
                let field = gtk::Entry::new();
                field.add_css_class("inline-rename");
                accessibility::set_label(&field, "Rename");
                field.set_hexpand(true);
                field.set_width_chars(1);
                field.set_visible(false);
                row.append(&labels);
                row.append(&field);
                Self {
                    widget: row,
                    icon,
                    edit: EditWidgets::new(&field, &labels),
                    labels: Labels::Rows { name, origin },
                }
            }
            ResultKind::Icons { thumbnail_size } => {
                let card = icons_cell::new_card(thumbnail_size.get());
                let (icon, name) = icons_cell::parts(&card).expect("icon card parts");
                let origin = icons_cell::details_label(&card).expect("icon card details");
                let field = icons_cell::ensure_rename_field(&card).expect("icon card editor");
                Self {
                    widget: card,
                    icon,
                    edit: EditWidgets::new(&field, &name),
                    labels: Labels::Icons { name, origin },
                }
            }
        }
    }

    pub(super) fn rename_label(&self) -> gtk::Widget {
        match &self.labels {
            Labels::Rows { name, .. } => name.clone().upcast(),
            Labels::Icons { name, .. } => name.clone().upcast(),
        }
    }

    pub(super) fn bind(
        &self,
        kind: &ResultKind,
        item: &gtk::ListItem,
        result: &SearchItem,
        root: &Path,
        recursive: bool,
    ) {
        let entry = browser::search_result_entry(result);
        self.edit.bind(&entry.location);
        accessibility::describe_entry(item, &result.name, Some(&entry));
        self.edit.display.set_visible(!self.edit.is_editing());
        self.edit.field.set_visible(self.edit.is_editing());
        let size = match kind {
            ResultKind::Rows => 17,
            ResultKind::Icons { thumbnail_size } => thumbnail_size.get(),
        };
        let path = relative_result_path(root, &result.path);
        match &self.labels {
            Labels::Rows { name, origin } => {
                name.set_text(&result.name);
                origin.set_text(&path);
                origin.set_visible(recursive);
                self.widget.set_tooltip_text(Some(&path));
            }
            Labels::Icons { name, origin } => {
                icons_cell::set_slot(&self.widget, size);
                name.set_text(Some(&result.name));
                name.set_tooltip_text(Some(&result.name));
                origin.add_css_class("file-search-path");
                origin.set_text(&path);
                origin.set_visible(recursive);
            }
        }
        if result.is_directory {
            thumbnail::show_customized_icon(
                &self.icon,
                &result.path,
                crate::assets::icons::FOLDER,
                size,
            );
        } else {
            thumbnail::set_thumbnail_or_icon_for_path(
                &self.icon,
                &result.path,
                crate::assets::icons::DOCUMENTS,
                size,
                size,
            );
        }
        browser::set_cut_result_style(&self.widget, &Location::local(&result.path));
    }
}

pub(super) fn relative_result_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}
