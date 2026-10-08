// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn install_command_can_be_selected_and_copied() {
    crate::test_support::gtk_test(
        "ui::controls::tests::install_command_can_be_selected_and_copied",
        || {
            let command = "sudo pacman -S --needed dosfstools ntfs-3g exfatprogs";
            let control = copyable_command(command);
            let field = control
                .child()
                .expect("command field")
                .downcast::<gtk::Entry>()
                .expect("selectable command entry");
            assert!(!field.is_editable());
            field.select_region(0, -1);
            assert_eq!(field.selection_bounds(), Some((0, command.len() as i32)));

            let mut child = control.first_child();
            let copy = loop {
                let widget = child.expect("copy command action");
                if widget.tooltip_text().as_deref() == Some("Copy install command") {
                    break widget.downcast::<gtk::Button>().expect("copy button");
                }
                child = widget.next_sibling();
            };
            copy.emit_clicked();
            let clipboard = gtk::gdk::Display::default()
                .expect("isolated GTK display")
                .clipboard();
            let copied = glib::MainContext::default()
                .block_on(clipboard.read_text_future())
                .expect("copied install command");
            assert_eq!(copied.as_deref(), Some(command));
            assert_eq!(
                copy.tooltip_text().as_deref(),
                Some("Install command copied")
            );
        },
    );
}

#[test]
fn character_counter_tracks_edits_and_character_limits() {
    crate::test_support::gtk_test(
        "ui::controls::tests::character_counter_tracks_edits_and_character_limits",
        || {
            let field = FormTextField::with_character_limit(11);
            assert_eq!(field.remaining.text(), "11 chars remaining");
            for (text, remaining) in [
                ("ARCH_202609", "0 chars remaining"),
                ("ARCH", "7 chars remaining"),
                ("é猫", "9 chars remaining"),
                ("", "11 chars remaining"),
            ] {
                field.entry.set_text(text);
                assert_eq!(field.remaining.text(), remaining);
            }

            field.entry.set_text("abcdefghijklmnop");
            assert_eq!(field.entry.text(), "abcdefghijk");
            assert_eq!(field.remaining.text(), "0 chars remaining");

            field.entry.set_max_length(32);
            assert_eq!(field.remaining.text(), "21 chars remaining");
            field.entry.set_text("NTFS");
            assert_eq!(field.remaining.text(), "28 chars remaining");

            field.entry.set_max_length(0);
            assert!(!field.remaining.is_visible());
            field.entry.set_max_length(11);
            assert!(field.remaining.is_visible());
            assert_eq!(field.remaining.text(), "7 chars remaining");
        },
    );
}

#[test]
fn dialog_text_keeps_paragraphs_and_unspaced_scripts_intact() {
    assert_eq!(
        dialog_text("0 件完了、1 件未処理。\n\n完了した変更は元に戻されていません。"),
        "0 件完了、1 件未処理。\n\n完了した変更は元に戻されていません。"
    );
    assert_eq!(
        dialog_text("  Completed  with errors:\n\n\n• a.txt:  denied\n• b.txt\n"),
        "Completed with errors:\n\n• a.txt: denied\n• b.txt"
    );
    let long_run = "アーカイブ".repeat(30);
    assert_eq!(dialog_text(&long_run), long_run);
    assert_eq!(
        dialog_text("Impossible de créer « a\u{a0}b »\u{a0}:  1\u{202f}234\u{2007}éléments"),
        "Impossible de créer « a\u{a0}b »\u{a0}: 1\u{202f}234\u{2007}éléments"
    );
}

#[test]
fn korean_words_are_kept_whole_except_long_runs() {
    let text = "다음 중 하나를 설치하세요: readonly에 a b";
    let words = korean_word_ranges(text)
        .into_iter()
        .map(|range| &text[range])
        .collect::<Vec<_>>();
    assert_eq!(words, ["다음", "하나를", "설치하세요:", "readonly에"]);
    assert!(korean_word_ranges(&"가나다".repeat(5)).is_empty());
    assert!(korean_word_ranges("Cannot create “readonly”").is_empty());
}
