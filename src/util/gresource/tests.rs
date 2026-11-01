// SPDX-License-Identifier: MIT

use gtk::{gio, glib, prelude::*};

use super::with_file;

#[test]
fn a_bundled_file_reads_back_by_path_listing_and_uri() {
    for (folder, name, data) in [
        ("/", "top.xml", b"<top/>".as_slice()),
        (
            "/io/github/lgse/Strata/generated/source-styles/",
            "strata-current.xml",
            b"<scheme/>",
        ),
        ("/a/", "caf\u{e9}.xml", b""),
    ] {
        let resource = with_file(folder, name, data).expect("valid bundle");
        let path = format!("{folder}{name}");
        let found = resource
            .lookup_data(&path, gio::ResourceLookupFlags::NONE)
            .expect("bundled file");
        assert_eq!(&*found, data, "{path}");
        assert_eq!(
            resource
                .enumerate_children(folder, gio::ResourceLookupFlags::NONE)
                .expect("bundled folder"),
            [glib::GString::from(name)],
            "{path}"
        );
        assert!(
            resource
                .lookup_data(
                    &format!("{folder}other.xml"),
                    gio::ResourceLookupFlags::NONE
                )
                .is_err()
        );

        gio::resources_register(&resource);
        let (contents, _) = gio::File::for_uri(&format!("resource://{path}"))
            .load_contents(gio::Cancellable::NONE)
            .expect("file read through resource://");
        gio::resources_unregister(&resource);
        assert_eq!(&*contents, data, "{path}");
    }
}
