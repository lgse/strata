// SPDX-License-Identifier: MIT

use rune::{Key, Mods, Tx};
use strata_core::model::{FileEntry, Location};
use strata_fs::services::{DocumentLayout, DocumentUnit, DocumentUnitKind, PreviewContent};

use super::{
    super::session,
    MODULE, Viewer, disable_auto_preview, focus_view,
    provider::{Recorded, install, listed_with, png},
    select,
};
use crate::{
    App,
    host::offscreen::Offscreen,
    test_support::{PATIENCE, hosted, send_key},
};

fn paragraph() -> PreviewContent {
    let text = "Quarterly report".to_owned();
    PreviewContent::Rendered {
        document: DocumentLayout {
            units: vec![DocumentUnit {
                kind: DocumentUnitKind::Paragraph,
                copy_text: text.clone(),
                text,
                spans: Vec::new(),
                wrap: true,
                first: true,
                last: true,
            }],
        },
        warnings: Vec::new(),
    }
}

fn entry_named(host: &Offscreen, name: &str) -> FileEntry {
    let browser = host.app().browser();
    (0..)
        .map_while(|position| browser.entry_at(0, position))
        .find(|entry| entry.display_name == name)
        .expect("the fixture's file")
}

fn shows(host: &Offscreen, viewer: &str) -> bool {
    matches!(
        (&host.app().preview.viewer, viewer),
        (Viewer::Image(_), "image") | (Viewer::Document(_), "document")
    )
}

#[test]
fn a_quick_move_off_a_viewer_lands_its_late_reports_in_the_spare_state() {
    hosted(
        MODULE,
        "spare::a_quick_move_off_a_viewer_lands_its_late_reports_in_the_spare_state",
        |mut host| {
            type Case = (&'static str, fn() -> PreviewContent, [&'static str; 2]);
            let cases: [Case; 2] = [
                (
                    "image",
                    || PreviewContent::Rasterized { png: png(64, 64) },
                    ["a-first.png", "a-second.png"],
                ),
                ("document", paragraph, ["a-third.md", "a-first.png"]),
            ];
            let provider = Recorded::default();
            install(&mut host, &provider);
            let files = [
                ("a-first.png", "a"),
                ("a-second.png", "b"),
                ("a-third.md", "c"),
            ];
            let folder = listed_with(&mut host, &files);
            disable_auto_preview(&mut host);
            for (viewer, content, [from, to]) in cases {
                let asked = provider.count();
                select(&mut host, &Location::local(folder.path().join(from)));
                focus_view(&mut host);
                if !host.app().preview.drawer.is_open() {
                    send_key(
                        &mut host,
                        rune::KeyEvent {
                            key: Key::Text(" ".to_owned()),
                            mods: Mods::default(),
                        },
                    );
                }
                let asked_now = |_: &Offscreen| provider.count() > asked;
                assert!(
                    host.run_until(PATIENCE, asked_now),
                    "{viewer}: the first file's load"
                );
                provider.answer(provider.count() - 1, content());
                // One frame: the view is built and its first layout report is queued.
                while !shows(&host, viewer) {
                    host.turn();
                }
                // The move to the next file, heard before the frame's reports are dispatched.
                let next = entry_named(&host, to);
                host.runtime().update(move |app: &mut App, tx: &mut Tx| {
                    session::show(app, next, None, tx);
                });
                while host.turn() {}
                assert!(
                    !shows(&host, viewer),
                    "{viewer}: the move replaced the view"
                );
            }
        },
    );
}
