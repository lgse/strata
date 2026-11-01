// SPDX-License-Identifier: MIT

use super::*;

const FIXTURE: &str = "/tmp/strata-clipboard-fixture.txt";

fn file_provider() -> gdk::ContentProvider {
    let files = gdk::FileList::from_array(&[gtk::gio::File::for_path(FIXTURE)]);
    gdk::ContentProvider::for_value(&files.to_value())
}

fn assert_paste(footer: &ShortcutFooter, available: bool, why: &str) {
    settle();
    assert_eq!(footer.paste.is_visible(), available, "{why}");
    assert_eq!(footer.widget().is_visible(), available, "{why}");
}

fn files_and_uri_lists_enable_paste_while_text_does_not(
    footer: &ShortcutFooter,
    clipboard: &gdk::Clipboard,
) {
    assert_paste(footer, true, "existing files on the clipboard enable paste");
    clipboard.set_text("plain text is not a file clipboard");
    assert_paste(footer, false, "plain text is not a file clipboard");
    let uri = gdk::ContentProvider::for_bytes(
        "text/uri-list",
        &glib::Bytes::from_static(b"file:///tmp/strata-clipboard-fixture.txt\r\n"),
    );
    clipboard.set_content(Some(&uri)).expect("URI clipboard");
    assert_paste(footer, true, "external URI lists also enable paste");
}

fn newer_cleared_and_empty_clipboards_withdraw_paste(
    footer: &ShortcutFooter,
    clipboard: &gdk::Clipboard,
) {
    clipboard
        .set_content(Some(&file_provider()))
        .expect("pending file clipboard");
    clipboard.set_text("newer clipboard replaces a pending file read");
    settle();
    assert!(!footer.paste.is_visible());
    clipboard
        .set_content(Some(&file_provider()))
        .expect("cut clipboard");
    settle();
    assert!(footer.paste.is_visible());
    clipboard
        .set_content(None::<&gdk::ContentProvider>)
        .expect("cleared clipboard");
    assert_paste(footer, false, "consuming a cut clears paste availability");
    let empty: Option<gdk::FileList> = None;
    clipboard
        .set_content(Some(&gdk::ContentProvider::for_value(&empty.to_value())))
        .expect("empty file clipboard");
    settle();
    assert!(!footer.paste.is_visible());
}

#[test]
fn paste_availability_tracks_file_clipboard() {
    crate::test_support::gtk_test(
        "ui::shortcut_footer::tests::clipboard::paste_availability_tracks_file_clipboard",
        || {
            let clipboard = gdk::Display::default().expect("test display").clipboard();
            clipboard
                .set_content(Some(&file_provider()))
                .expect("file clipboard");
            let footer = ShortcutFooter::new(BrowserMode::Columns);
            let handler = footer.connect_clipboard(&clipboard);
            let manager = crate::ui::preferences::PreferenceManager::shared();
            manager.set_show_keybinding_hints(false);
            footer.bind_preferences(&manager);
            footer.assert_hints_visible(false);
            files_and_uri_lists_enable_paste_while_text_does_not(&footer, &clipboard);
            newer_cleared_and_empty_clipboards_withdraw_paste(&footer, &clipboard);
            clipboard.disconnect(handler);
            clipboard
                .set_content(None::<&gdk::ContentProvider>)
                .expect("fixture cleanup");
        },
    );
}
