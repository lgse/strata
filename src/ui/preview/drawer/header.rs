// SPDX-License-Identifier: MIT

use super::*;

/// The preview header: file identity on the drag handle, then the actions.
pub(super) struct Header {
    pub(super) row: gtk::Box,
    pub(super) handle: gtk::Box,
    pub(super) icon: gtk::Image,
    pub(super) title: gtk::Label,
    pub(super) metadata: Metadata,
    pub(super) open: gtk::Button,
    pub(super) model_spinner: gtk::Spinner,
    pub(super) document_view_icon: gtk::Image,
    pub(super) document_view_button: gtk::Button,
    pub(super) wrap: gtk::ToggleButton,
    pub(super) close: gtk::Button,
    pub(super) find_button: gtk::Button,
}

pub(super) struct Metadata {
    pub(super) row: gtk::Box,
    pub(super) size: gtk::Label,
    pub(super) modified: gtk::Label,
    pub(super) content_type: gtk::Label,
}

impl Header {
    pub(super) fn new(allow_external_open: bool) -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("preview-header");
        let icon = crate::assets::chrome_icon(crate::assets::icons::DOCUMENTS);
        let title = title_label();
        let open = open_button(allow_external_open);
        let model_spinner = model_spinner();
        let document_view_icon = crate::assets::primary_icon(crate::assets::icons::FILE_CODE, 16);
        let document_view_button = document_view_button(&document_view_icon);
        let wrap = wrap_button();
        let close = close_button();
        let handle = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        handle.add_css_class("preview-header-handle");
        handle.set_hexpand(true);
        handle.set_margin_end(8);
        handle.set_cursor_from_name(Some("grab"));
        handle.append(&icon);
        let heading = gtk::Box::new(gtk::Orientation::Vertical, 0);
        heading.set_hexpand(true);
        heading.set_valign(gtk::Align::Center);
        heading.append(&title);
        let metadata = Metadata::new();
        heading.append(&metadata.row);
        handle.append(&heading);
        row.append(&handle);
        let find_button = find::icon_button("Find in preview", crate::assets::icons::SEARCH);
        find_button.set_visible(false);
        row.append(&document_view_button);
        row.append(&find_button);
        row.append(&model_spinner);
        row.append(&open);
        row.append(&wrap);
        row.append(&close);
        Self {
            row,
            handle,
            icon,
            title,
            metadata,
            open,
            model_spinner,
            document_view_icon,
            document_view_button,
            wrap,
            close,
            find_button,
        }
    }
}

impl Metadata {
    fn new() -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        row.add_css_class("preview-metadata");
        let size = metadata_value("Size");
        let modified = metadata_value("Modified");
        let content_type = metadata_value("Type");
        content_type.set_hexpand(true);
        row.append(&size);
        row.append(&gtk::Label::new(Some("·")));
        row.append(&modified);
        row.append(&gtk::Label::new(Some("·")));
        row.append(&content_type);
        Self {
            row,
            size,
            modified,
            content_type,
        }
    }
}

fn title_label() -> gtk::Label {
    let title = gtk::Label::new(None);
    title.add_css_class("preview-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    title.set_hexpand(true);
    title.set_xalign(0.0);
    title
}

fn open_button(allow_external_open: bool) -> gtk::Button {
    let open = gtk::Button::builder()
        .tooltip_text(crate::i18n::tr("Open in default application"))
        .valign(gtk::Align::Center)
        .build();
    super::super::super::accessibility::set_label(
        &open,
        &crate::i18n::tr("Open in default application"),
    );
    open.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::EXTERNAL_LINK,
    )));
    open.add_css_class("preview-header-action");
    open.set_visible(allow_external_open);
    open
}

fn model_spinner() -> gtk::Spinner {
    let model_spinner = gtk::Spinner::builder()
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    model_spinner.add_css_class("preview-spinner");
    model_spinner.add_css_class("preview-header-spinner");
    model_spinner
}

fn document_view_button(icon: &gtk::Image) -> gtk::Button {
    let button = gtk::Button::builder()
        .tooltip_text(crate::i18n::tr("View source"))
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    button.set_child(Some(icon));
    button.add_css_class("preview-header-action");
    super::super::super::accessibility::set_label(&button, &crate::i18n::tr("View source"));
    button
}

fn wrap_button() -> gtk::ToggleButton {
    let wrap = gtk::ToggleButton::builder()
        .tooltip_text(crate::i18n::tr("Toggle word wrap"))
        .valign(gtk::Align::Center)
        .build();
    super::super::super::accessibility::set_label(&wrap, &crate::i18n::tr("Toggle word wrap"));
    wrap.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::WRAP_TEXT,
    )));
    wrap.add_css_class("preview-header-action");
    wrap.set_visible(false);
    wrap
}

fn close_button() -> gtk::Button {
    let close = gtk::Button::builder().valign(gtk::Align::Center).build();
    super::super::super::preferences::PreferenceManager::shared().bind_preference(
        &close,
        super::super::super::preferences::PreferenceManager::shown_keys,
        |close, _| {
            let label = crate::ui::shortcut_reference::with_keys(
                "Close preview (%{keys})",
                keys::Keys::One(PREVIEW.shown()),
            );
            close.set_tooltip_text(Some(&label));
            super::super::super::accessibility::set_label(close, &label);
        },
    );
    close.set_child(Some(&crate::assets::chrome_icon(
        crate::assets::icons::PANEL_RIGHT_CLOSE,
    )));
    close.add_css_class("preview-close");
    close.add_css_class("preview-header-action");
    close
}
