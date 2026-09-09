// SPDX-License-Identifier: MIT

use std::{cell::RefCell, rc::Rc};

use gtk::{gdk, prelude::*};

use crate::ui::{
    controls::form_entry,
    theme::{ThemeManager, ThemeTokens},
};

pub(super) fn theme_editor(manager: Rc<ThemeManager>) -> (gtk::Revealer, gtk::FlowBox) {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    panel.add_css_class("theme-editor");
    panel.append(&editor_header());
    let name = form_entry();
    name.set_placeholder_text(Some("Theme name"));
    panel.append(&name);

    let values = Rc::new(RefCell::new(manager.starter_tokens()));
    let fields = theme_color_fields(&manager, &values);
    panel.append(&fields);

    let error = gtk::Label::new(None);
    error.add_css_class("theme-editor-error");
    error.set_xalign(0.0);
    error.set_visible(false);
    panel.append(&error);

    let revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .child(&panel)
        .build();
    panel.append(&editor_actions(
        manager,
        ThemeEditorForm {
            name,
            values,
            error,
            revealer: revealer.clone(),
        },
    ));
    (revealer, fields)
}

fn editor_header() -> gtk::Box {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some("Add a theme"));
    title.add_css_class("settings-option-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    header
}

fn theme_color_fields(
    manager: &Rc<ThemeManager>,
    values: &Rc<RefCell<ThemeTokens>>,
) -> gtk::FlowBox {
    let fields = gtk::FlowBox::builder()
        .column_spacing(18)
        .row_spacing(10)
        .max_children_per_line(4)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    fields.add_css_class("theme-color-fields");
    for (label_text, field) in ColorField::ALL {
        fields.insert(&color_field_row(label_text, field, manager, values), -1);
    }
    fields
}

fn color_field_row(
    label_text: &str,
    field: ColorField,
    manager: &Rc<ThemeManager>,
    values: &Rc<RefCell<ThemeTokens>>,
) -> gtk::Box {
    let field_row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    let label = gtk::Label::new(Some(label_text));
    label.set_xalign(0.0);
    let dialog = gtk::ColorDialog::builder()
        .title(format!("Choose {label_text}"))
        .with_alpha(false)
        .build();
    let picker = gtk::ColorDialogButton::new(Some(dialog));
    picker.add_css_class("theme-color-picker");
    if let Ok(color) = gdk::RGBA::parse(field.slot(&mut values.borrow_mut()).as_str()) {
        picker.set_rgba(&color);
    }
    let values_for_color = values.clone();
    let manager_for_color = manager.clone();
    picker.connect_rgba_notify(move |picker| {
        *field.slot(&mut values_for_color.borrow_mut()) = picker.rgba().to_string();
        manager_for_color.preview(&values_for_color.borrow());
    });
    field_row.append(&picker);
    field_row.append(&label);
    field_row
}

struct ThemeEditorForm {
    name: gtk::Entry,
    values: Rc<RefCell<ThemeTokens>>,
    error: gtk::Label,
    revealer: gtk::Revealer,
}

fn editor_actions(manager: Rc<ThemeManager>, form: ThemeEditorForm) -> gtk::Box {
    let ThemeEditorForm {
        name,
        values,
        error,
        revealer,
    } = form;
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.add_css_class("action-dialog-cancel");
    let save = gtk::Button::with_label("Add theme");
    save.add_css_class("action-dialog-confirm");
    actions.append(&cancel);
    actions.append(&save);
    let hidden = revealer.clone();
    let manager_for_cancel = manager.clone();
    cancel.connect_clicked(move |_| {
        manager_for_cancel.cancel_preview();
        hidden.set_reveal_child(false);
    });
    save.connect_clicked(move |_| {
        let mut tokens = values.borrow().clone();
        tokens.name = name.text().trim().to_owned();
        match manager.save_custom_theme(tokens) {
            Ok(_) => {
                error.set_visible(false);
                revealer.set_reveal_child(false);
            }
            Err(message) => {
                error.set_text(&message.to_string());
                error.set_visible(true);
            }
        }
    });
    actions
}

#[derive(Clone, Copy)]
enum ColorField {
    Background,
    Surface,
    Text,
    Accent,
    Danger,
    Muted,
    Highlight,
    Border,
    DimText,
}

impl ColorField {
    const ALL: [(&'static str, Self); 9] = [
        ("Background", Self::Background),
        ("Surface", Self::Surface),
        ("Text", Self::Text),
        ("Accent", Self::Accent),
        ("Danger", Self::Danger),
        ("Muted", Self::Muted),
        ("Highlight", Self::Highlight),
        ("Border", Self::Border),
        ("Dim text", Self::DimText),
    ];

    fn slot(self, tokens: &mut ThemeTokens) -> &mut String {
        match self {
            Self::Background => &mut tokens.background,
            Self::Surface => &mut tokens.surface,
            Self::Text => &mut tokens.text,
            Self::Accent => &mut tokens.accent,
            Self::Danger => &mut tokens.danger,
            Self::Muted => &mut tokens.muted,
            Self::Highlight => &mut tokens.highlight,
            Self::Border => &mut tokens.border,
            Self::DimText => &mut tokens.dim_text,
        }
    }
}
