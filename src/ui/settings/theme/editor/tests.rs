// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::gtk_test;
use sourceview5::prelude::*;

fn foreground(buffer: &sourceview5::Buffer, style: &str) -> Option<glib::GString> {
    buffer
        .style_scheme()
        .expect("installed source scheme")
        .style(style)
        .expect("syntax style")
        .foreground()
}

#[test]
fn syntax_pickers_preview_cancel_and_save_to_custom_theme() {
    gtk_test(
        "ui::settings::theme::editor::tests::syntax_pickers_preview_cancel_and_save_to_custom_theme",
        || {
            let manager = ThemeManager::shared();
            manager.set_follow_omarchy(false);
            manager.select_theme("azure-glow");
            let buffers = [
                sourceview5::Buffer::new(None),
                sourceview5::Buffer::new(None),
            ];
            for buffer in &buffers {
                crate::ui::theme::register_source_buffer(buffer);
            }
            let original = foreground(&buffers[0], "def:string");
            let ThemeEditor {
                revealer: editor,
                fields,
                ..
            } = theme_editor(manager.clone());
            let mut child = fields.first_child();
            while let Some(wrapper) = child {
                let row = wrapper.first_child().expect("color field row");
                let label = row
                    .last_child()
                    .expect("field label")
                    .downcast::<gtk::Label>()
                    .expect("label widget");
                if label.text().starts_with("Syntax ") {
                    let picker = row
                        .first_child()
                        .expect("color picker")
                        .downcast::<gtk::ColorDialogButton>()
                        .expect("picker widget");
                    picker.set_rgba(&gdk::RGBA::parse("#123abc").expect("fixture color"));
                }
                child = wrapper.next_sibling();
            }
            for buffer in &buffers {
                for style in [
                    "def:statement",
                    "def:string",
                    "def:constant",
                    "def:type",
                    "def:preprocessor",
                ] {
                    assert_eq!(foreground(buffer, style).as_deref(), Some("#123abc"));
                }
            }
            manager.cancel_preview();
            for buffer in &buffers {
                assert_eq!(foreground(buffer, "def:string"), original);
            }
            let panel = editor.child().expect("editor panel");
            let name = panel
                .first_child()
                .expect("editor header")
                .next_sibling()
                .expect("name entry")
                .downcast::<gtk::Entry>()
                .expect("entry widget");
            name.set_text("Syntax fixture");
            let save = panel
                .last_child()
                .expect("editor actions")
                .last_child()
                .expect("save action")
                .downcast::<gtk::Button>()
                .expect("save button");
            save.emit_clicked();
            let source = std::fs::read_to_string(
                glib::user_config_dir().join("strata/themes/syntax-fixture.toml"),
            )
            .expect("saved custom theme");
            let restored: ThemeTokens = toml::from_str(&source).expect("valid custom theme");
            for color in [
                restored.syntax_keyword,
                restored.syntax_string,
                restored.syntax_constant,
                restored.syntax_type,
                restored.syntax_preprocessor,
            ] {
                assert_eq!(color.as_deref(), Some("#123abc"));
            }
            let rebuilt = sourceview5::Buffer::new(None);
            crate::ui::theme::register_source_buffer(&rebuilt);
            assert_eq!(
                foreground(&rebuilt, "def:string").as_deref(),
                Some("#123abc")
            );
        },
    );
}

fn accent_picker(fields: &gtk::FlowBox) -> gtk::ColorDialogButton {
    let mut child = fields.first_child();
    while let Some(wrapper) = child {
        let row = wrapper.first_child().expect("color field row");
        let label = row
            .last_child()
            .and_downcast::<gtk::Label>()
            .expect("field label");
        if label.text() == "Accent" {
            return row
                .first_child()
                .and_downcast::<gtk::ColorDialogButton>()
                .expect("accent picker");
        }
        child = wrapper.next_sibling();
    }
    panic!("accent field");
}

#[test]
fn dismissing_the_editor_discards_the_preview_and_resets_the_form() {
    gtk_test(
        "ui::settings::theme::editor::tests::dismissing_the_editor_discards_the_preview_and_resets_the_form",
        || {
            let manager = ThemeManager::shared();
            manager.set_follow_omarchy(false);
            manager.select_theme("azure-glow");
            let saved = manager.active_model_palette();
            let saved_accent = gdk::RGBA::parse(
                manager
                    .current_tokens()
                    .expect("selected theme tokens")
                    .accent
                    .as_str(),
            )
            .expect("saved accent");
            let ThemeEditor {
                revealer: editor,
                fields,
                ..
            } = theme_editor(manager.clone());
            editor.set_reveal_child(true);
            let panel = editor.child().expect("editor panel");
            let name = panel
                .first_child()
                .and_then(|header| header.next_sibling())
                .and_downcast::<gtk::Entry>()
                .expect("name entry");
            let cancel = panel
                .last_child()
                .and_then(|actions| actions.first_child())
                .and_downcast::<gtk::Button>()
                .expect("cancel button");

            accent_picker(&fields).set_rgba(&gdk::RGBA::parse("#13579b").expect("fixture"));
            name.set_text("Draft");
            assert_eq!(manager.active_model_palette().accent, 0x13579b);
            cancel.emit_clicked();

            let mut failures = Vec::new();
            if manager.active_model_palette() != saved {
                failures.push("the preview is still applied".to_owned());
            }
            if editor.reveals_child() {
                failures.push("the editor is still revealed".to_owned());
            }
            if accent_picker(&fields).rgba() != saved_accent {
                failures.push(format!(
                    "the accent picker keeps {} instead of the selected theme's {}",
                    accent_picker(&fields).rgba(),
                    saved_accent
                ));
            }
            if !name.text().is_empty() {
                failures.push(format!("the name entry keeps {:?}", name.text()));
            }
            assert!(
                failures.is_empty(),
                "Cancel must discard the preview and reset the form:\n{}",
                failures.join("\n")
            );
        },
    );
}

#[test]
fn dismissing_an_editor_keeps_another_editors_preview() {
    gtk_test(
        "ui::settings::theme::editor::tests::dismissing_an_editor_keeps_another_editors_preview",
        || {
            let manager = ThemeManager::shared();
            manager.set_follow_omarchy(false);
            manager.select_theme("azure-glow");
            for superseded in [false, true] {
                let editing = theme_editor(manager.clone());
                let other = theme_editor(manager.clone());
                (editing.reveal)();
                (other.reveal)();
                if superseded {
                    accent_picker(&other.fields)
                        .set_rgba(&gdk::RGBA::parse("#2468ac").expect("fixture"));
                }
                accent_picker(&editing.fields)
                    .set_rgba(&gdk::RGBA::parse("#13579b").expect("fixture"));
                assert!(manager.is_previewing());

                (other.dismiss)();
                assert!(
                    manager.is_previewing(),
                    "superseded: {superseded}: another window's preview stays applied"
                );
                assert!(!other.revealer.reveals_child());

                (editing.dismiss)();
                assert!(!manager.is_previewing(), "superseded: {superseded}");
            }
        },
    );
}
