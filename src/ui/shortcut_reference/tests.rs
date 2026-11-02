// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::keys::{Keymap, use_keymap};

const MODES: [BrowserMode; 3] = [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List];

const BACK_HISTORY: &str = "List the back history (also right-click or hold)";
const NEXT_RENAME: &str = "Rename the next / previous item";

fn rows_for(sections: &[ReferenceSection], action: &str) -> usize {
    sections
        .iter()
        .flat_map(|section| &section.rows)
        .filter(|row| row.action == action)
        .count()
}

/// The 10xer window and every chooser request.
fn tenxer_choosers() -> Vec<Option<ChooserScope>> {
    let requests = [
        ChooserRequest::Files,
        ChooserRequest::Folders,
        ChooserRequest::SaveFile,
        ChooserRequest::SaveFiles,
    ];
    std::iter::once(None)
        .chain(requests.into_iter().flat_map(|request| {
            [false, true].map(|multiple| Some(ChooserScope { request, multiple }))
        }))
        .collect()
}

/// The Back button lives in the Icons and List pane header, which Columns lacks
/// and 10xer hides. Tab moves an inline rename in either map, the 10xer
/// chooser's context-menu rename included.
#[test]
fn rows_appear_once_where_their_feature_exists() {
    let _keymap = use_keymap(Keymap::defaults());
    let mut wrong = Vec::new();
    let mut expect = |scope: String, sections: &[ReferenceSection], action, expected| {
        let found = rows_for(sections, action);
        if found != expected {
            wrong.push(format!(
                "{scope}: {found} {action:?} rows, expected {expected}"
            ));
        }
    };
    // Expected rows in Columns, Icons and List: default map, then 10xer.
    for (action, default, tenxer) in [
        (BACK_HISTORY, [0, 1, 1], [0, 0, 0]),
        (NEXT_RENAME, [1, 1, 1], [1, 1, 1]),
    ] {
        for ((mode, default), tenxer) in MODES.into_iter().zip(default).zip(tenxer) {
            expect(
                format!("default {mode:?}"),
                &default_sections(mode),
                action,
                default,
            );
            for chooser in tenxer_choosers() {
                let sections = tenxer_sections(mode, chooser);
                expect(
                    format!("10xer {mode:?} {chooser:?}"),
                    &sections,
                    action,
                    tenxer,
                );
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
