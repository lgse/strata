// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn web_links_round_trip_as_data_only_shortcuts() {
    for address in [
        "https://example.org/path?q=a%20b&lang=日本語#part",
        "http://localhost:8765/image.png",
        "https://[::1]/",
    ] {
        let link = WebLink::parse(address).expect("web link");
        let data = String::from_utf8(link.desktop_entry()).expect("desktop entry");
        assert_eq!(WebLink::from_desktop_entry(&data), Ok(Some(link)));
        let entry = KeyFile::new();
        entry
            .load_from_data(&data, KeyFileFlags::NONE)
            .expect("key file");
        assert!(!entry.has_key("Desktop Entry", "Exec").unwrap_or(false));
    }
}

#[test]
fn web_links_reject_commands_credentials_and_invalid_addresses() {
    for address in [
        "javascript:alert(1)",
        "data:text/html,<script/>",
        "file:///tmp/program",
        "ftp://example.org/file",
        "https:///",
        "https://user:secret@example.org/",
        "https://example.org/\nExec=touch /tmp/owned",
        "https://example.org/a b",
        "https://example.org/%GG",
    ] {
        assert!(WebLink::parse(address).is_none(), "{address:?}");
        let data = format!("[Desktop Entry]\nType=Link\nURL={address}\n");
        assert!(WebLink::from_desktop_entry(&data).is_err(), "{address:?}");
    }
    for data in [
        "[Desktop Entry]\nType=Link\nURL=https://example.org/\nExec=touch /tmp/owned\n",
        "[Desktop Entry]\nType=Link\n",
        "[Desktop Entry]\nType=Link\nURL=https://example.org/\nnot a key\n",
    ] {
        assert!(WebLink::from_desktop_entry(data).is_err(), "{data:?}");
    }
}

#[test]
fn application_launchers_are_not_interpreted_as_web_links() {
    assert_eq!(
        WebLink::from_desktop_entry(
            "[Desktop Entry]\nType=Application\nExec=example\nURL=https://example.org/\n"
        ),
        Ok(None)
    );
}
