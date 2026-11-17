// SPDX-License-Identifier: MIT

//! 10xer's keys on an open preview (`tenxer_preview`, `scroll_open_preview` and
//! `preview_to_top` in `src/ui/window/keyboard/preview.rs`): `J` and `K` scroll it a line from
//! the listing or inside it, and `g g` takes a focused document or archive to its top. A15's
//! drawer keys (`l`, `h`, Escape, `i`, `/`) and Shift+Tab are the preview part's.

use rune::Tx;
use strata_desktop::keys::{
    KeyPress,
    bindings::{GO, GO_FIRST, SCROLL_PREVIEW_DOWN, SCROLL_PREVIEW_UP},
};

use super::{ChordKind, arm_default_chord, go};
use crate::{App, commands::Outcome, preview};

/// `J` and `K`, from the listing or the preview: claimed whether or not a preview shows, as
/// GTK's are.
pub(super) fn scroll(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    let lines = if press.is(SCROLL_PREVIEW_DOWN) {
        1
    } else if press.is(SCROLL_PREVIEW_UP) {
        -1
    } else {
        return None;
    };
    let _ = preview::scroll_lines(app, lines, tx);
    Some(Outcome::Ran)
}

/// The keys inside a focused preview.
pub(super) fn in_preview(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    if let Some(outcome) = scroll(app, press, tx) {
        return Some(outcome);
    }
    if press.is(GO) && preview::has_top(app) {
        arm_default_chord(app, ChordKind::PreviewTop);
        return Some(Outcome::Ran);
    }
    None
}

/// The step after `g` in a preview: `g` goes to its top; a place's key leaves the preview for
/// the listing and goes there, as `g` does from the listing.
pub(super) fn complete_top(app: &mut App, press: &KeyPress, tx: &mut Tx) -> bool {
    if press.is(GO_FIRST) {
        preview::to_top(app, tx);
        return true;
    }
    if !go::is_target(app, press) {
        return false;
    }
    preview::return_to_files(app);
    go::complete(app, press, tx)
}
