// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests;

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, prelude::*};

use crate::ui::{
    controls::form_entry,
    theme::{ThemeManager, ThemeTokens, color_to_hex},
};

pub(super) struct ThemeEditor {
    pub(super) revealer: gtk::Revealer,
    pub(super) fields: gtk::FlowBox,
    /// Opens the editor on the selected theme's colors.
    pub(super) reveal: Rc<dyn Fn()>,
    /// Discards the draft and its live preview, and collapses the editor.
    pub(super) dismiss: Rc<dyn Fn()>,
}

type ColorPickers = Vec<(ColorField, gtk::ColorDialogButton)>;

pub(super) fn theme_editor(manager: Rc<ThemeManager>) -> ThemeEditor {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 12);
    panel.add_css_class("theme-editor");
    panel.append(&editor_header());
    let name = form_entry();
    name.set_placeholder_text(Some(&crate::i18n::tr("Theme name")));
    panel.append(&name);

    let values = Rc::new(RefCell::new(starter_tokens(&manager)));
    let preview = PreviewState::default();
    let (fields, pickers) = theme_color_fields(&manager, &values, &preview);
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

    let reset: Rc<dyn Fn()> = {
        let manager = manager.clone();
        let values = values.clone();
        let name = name.clone();
        let error = error.clone();
        let preview = preview.clone();
        Rc::new(move || {
            let mut tokens = starter_tokens(&manager);
            preview.syncing.set(true);
            for (field, picker) in &pickers {
                if let Ok(color) = gdk::RGBA::parse(field.slot(&mut tokens).as_str()) {
                    picker.set_rgba(&color);
                }
            }
            preview.syncing.set(false);
            preview.applied.set(None);
            values.replace(tokens);
            name.set_text("");
            error.set_visible(false);
        })
    };
    let reveal: Rc<dyn Fn()> = {
        let reset = reset.clone();
        let revealer = revealer.clone();
        Rc::new(move || {
            // A second Add theme press keeps the open draft.
            if !revealer.reveals_child() {
                reset();
                revealer.set_reveal_child(true);
            }
        })
    };
    let dismiss: Rc<dyn Fn()> = {
        let manager = manager.clone();
        let revealer = revealer.clone();
        let preview = preview.clone();
        Rc::new(move || {
            // The preview is process-wide: only end the one this editor started.
            if let Some(generation) = preview.applied.take() {
                manager.cancel_preview_from(generation);
            }
            if revealer.reveals_child() {
                reset();
                revealer.set_reveal_child(false);
            }
        })
    };
    panel.append(&editor_actions(
        manager,
        ThemeEditorForm {
            name,
            values,
            error,
            revealer: revealer.clone(),
            dismiss: dismiss.clone(),
            preview,
        },
    ));
    ThemeEditor {
        revealer,
        fields,
        reveal,
        dismiss,
    }
}

/// `syncing` mutes the pickers while a reset sets them; `applied` holds the generation
/// of the preview this editor last started.
#[derive(Clone, Default)]
struct PreviewState {
    syncing: Rc<Cell<bool>>,
    applied: Rc<Cell<Option<u64>>>,
}

fn starter_tokens(manager: &ThemeManager) -> ThemeTokens {
    let mut tokens = manager.starter_tokens();
    tokens.initialize_syntax_colors();
    tokens
}

fn editor_header() -> gtk::Box {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let title = gtk::Label::new(Some(&crate::i18n::tr("Add a theme")));
    title.add_css_class("settings-option-title");
    title.set_xalign(0.0);
    title.set_hexpand(true);
    header.append(&title);
    header
}

fn theme_color_fields(
    manager: &Rc<ThemeManager>,
    values: &Rc<RefCell<ThemeTokens>>,
    preview: &PreviewState,
) -> (gtk::FlowBox, ColorPickers) {
    let fields = gtk::FlowBox::builder()
        .column_spacing(18)
        .row_spacing(10)
        .max_children_per_line(4)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .build();
    fields.add_css_class("theme-color-fields");
    let mut pickers = Vec::new();
    for (label_text, field) in ColorField::ALL {
        let (row, picker) = color_field_row(label_text, field, manager, values, preview);
        fields.insert(&row, -1);
        pickers.push((field, picker));
    }
    (fields, pickers)
}

fn color_field_row(
    label_text: &str,
    field: ColorField,
    manager: &Rc<ThemeManager>,
    values: &Rc<RefCell<ThemeTokens>>,
    preview: &PreviewState,
) -> (gtk::Box, gtk::ColorDialogButton) {
    let field_row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    let label_text = crate::i18n::tr(label_text);
    let label = gtk::Label::new(Some(&label_text));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    // The grid is homogeneous, so a character-wrapping label would shrink every field.
    label.add_css_class("settings-word-wrap");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::Word);
    let dialog = gtk::ColorDialog::builder()
        .title(rust_i18n::t!("%{label_text} color", label_text = label_text).into_owned())
        .with_alpha(false)
        .build();
    let picker = gtk::ColorDialogButton::new(Some(dialog));
    picker.add_css_class("theme-color-picker");
    if let Ok(color) = gdk::RGBA::parse(field.slot(&mut values.borrow_mut()).as_str()) {
        picker.set_rgba(&color);
    }
    let values_for_color = values.clone();
    let manager_for_color = manager.clone();
    let preview = preview.clone();
    picker.connect_rgba_notify(move |picker| {
        if preview.syncing.get() {
            return;
        }
        *field.slot(&mut values_for_color.borrow_mut()) = color_to_hex(&picker.rgba().to_string());
        if let Some(generation) = manager_for_color.preview(&values_for_color.borrow()) {
            preview.applied.set(Some(generation));
        }
    });
    field_row.append(&picker);
    field_row.append(&label);
    (field_row, picker)
}

struct ThemeEditorForm {
    name: gtk::Entry,
    values: Rc<RefCell<ThemeTokens>>,
    error: gtk::Label,
    revealer: gtk::Revealer,
    dismiss: Rc<dyn Fn()>,
    preview: PreviewState,
}

fn editor_actions(manager: Rc<ThemeManager>, form: ThemeEditorForm) -> gtk::Box {
    let ThemeEditorForm {
        name,
        values,
        error,
        revealer,
        dismiss,
        preview,
    } = form;
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label(&crate::i18n::tr("Cancel"));
    cancel.add_css_class("action-dialog-cancel");
    let save = gtk::Button::with_label(&crate::i18n::tr("Add theme"));
    save.add_css_class("action-dialog-confirm");
    actions.append(&cancel);
    actions.append(&save);
    cancel.connect_clicked(move |_| dismiss());
    save.connect_clicked(move |_| {
        let mut tokens = values.borrow().clone();
        tokens.name = name.text().trim().to_owned();
        match manager.save_custom_theme(tokens) {
            Ok(_) => {
                // Saving selects the theme, which ends the preview.
                preview.applied.set(None);
                error.set_visible(false);
                revealer.set_reveal_child(false);
            }
            Err(message) => {
                error.set_text(&save_error_text(&message));
                error.set_visible(true);
            }
        }
    });
    actions
}

fn save_error_text(error: &std::io::Error) -> String {
    // Validation errors carry an English catalog key; other errors come from the OS.
    if error.kind() == std::io::ErrorKind::InvalidInput {
        crate::i18n::tr(&error.to_string())
    } else {
        rust_i18n::t!(
            "Could not save the theme: %{error}",
            error = crate::services::io_error_detail(error)
        )
        .into_owned()
    }
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
    SyntaxKeyword,
    SyntaxString,
    SyntaxConstant,
    SyntaxType,
    SyntaxPreprocessor,
}

impl ColorField {
    const ALL: [(&'static str, Self); 14] = [
        ("Background", Self::Background),
        ("Surface", Self::Surface),
        ("Text", Self::Text),
        ("Accent", Self::Accent),
        ("Danger", Self::Danger),
        ("Muted", Self::Muted),
        ("Highlight", Self::Highlight),
        ("Border", Self::Border),
        ("Dim text / comments", Self::DimText),
        ("Syntax keywords", Self::SyntaxKeyword),
        ("Syntax strings", Self::SyntaxString),
        ("Syntax constants", Self::SyntaxConstant),
        ("Syntax types", Self::SyntaxType),
        ("Syntax preprocessor", Self::SyntaxPreprocessor),
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
            Self::SyntaxKeyword => tokens
                .syntax_keyword
                .as_mut()
                .expect("initialized syntax color"),
            Self::SyntaxString => tokens
                .syntax_string
                .as_mut()
                .expect("initialized syntax color"),
            Self::SyntaxConstant => tokens
                .syntax_constant
                .as_mut()
                .expect("initialized syntax color"),
            Self::SyntaxType => tokens
                .syntax_type
                .as_mut()
                .expect("initialized syntax color"),
            Self::SyntaxPreprocessor => tokens
                .syntax_preprocessor
                .as_mut()
                .expect("initialized syntax color"),
        }
    }
}
