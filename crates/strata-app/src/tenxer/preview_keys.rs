// SPDX-License-Identifier: MIT

use std::time::Duration;

use rune::Tx;
use strata_desktop::keys::{
    KeyPress, Match, Modifiers,
    bindings::{
        DOWN, END_KEY, FIRST_ITEM, FIRST_ITEM_UP, GO, GO_FIRST, HALF_PAGE_DOWN, HALF_PAGE_UP,
        HOME_KEY, LAST_ITEM, LAST_ITEM_DOWN, LEFT, PAGE_DOWN, PAGE_DOWN_KEY, PAGE_UP, PAGE_UP_KEY,
        SCROLL_PREVIEW_DOWN, SCROLL_PREVIEW_UP, UP,
    },
};

use super::{ChordKind, arm_default_chord, go};
use crate::{
    App,
    commands::{Outcome, Situation},
    preview::{self, DocumentScroll},
};

pub(super) fn scroll(app: &mut App, press: &KeyPress, tx: &mut Tx) -> Option<Outcome> {
    let lines = if press.is(SCROLL_PREVIEW_DOWN) {
        1
    } else if press.is(SCROLL_PREVIEW_UP) {
        -1
    } else {
        return None;
    };
    if !preview::is_media_preview(app) {
        let _ = preview::scroll_lines(app, lines, tx);
    }
    Some(Outcome::Ran)
}

/// The keys inside a focused preview.
pub(super) fn in_preview(
    app: &mut App,
    press: &KeyPress,
    at: &Situation,
    tx: &mut Tx,
) -> Option<Outcome> {
    if let Some(outcome) = scroll(app, press, tx) {
        return Some(outcome);
    }
    if press.is(GO) && preview::has_top(app) {
        arm_default_chord(app, ChordKind::PreviewTop);
        return Some(Outcome::Ran);
    }
    if !preview::in_document_body(app, at) {
        return None;
    }
    if press.is(LEFT.or_alternates()) {
        preview::return_to_files(app);
        tx.wake_after(Duration::ZERO);
        return Some(Outcome::Ran);
    }
    let motion = document_motion(press)?;
    let _ = preview::scroll_document(app, motion, tx);
    Some(Outcome::Ran)
}

fn document_motion(press: &KeyPress) -> Option<DocumentScroll> {
    let down_or_j = DOWN.or_alternates();
    let up_or_k = UP.or_alternates();
    let page_up_any_shift = PAGE_UP_KEY.ignoring(Modifiers::SHIFT_MASK);
    let page_down_any_shift = PAGE_DOWN_KEY.ignoring(Modifiers::SHIFT_MASK);
    let motions: [(&dyn Match, DocumentScroll); 14] = [
        (&HALF_PAGE_UP, DocumentScroll::HalfPage(-1)),
        (&HALF_PAGE_DOWN, DocumentScroll::HalfPage(1)),
        (&PAGE_UP, DocumentScroll::Page(-1)),
        (&PAGE_DOWN, DocumentScroll::Page(1)),
        (&FIRST_ITEM_UP, DocumentScroll::Start),
        (&LAST_ITEM_DOWN, DocumentScroll::End),
        (&down_or_j, DocumentScroll::Line(1)),
        (&up_or_k, DocumentScroll::Line(-1)),
        (&page_up_any_shift, DocumentScroll::Page(-1)),
        (&page_down_any_shift, DocumentScroll::Page(1)),
        (&HOME_KEY, DocumentScroll::Start),
        (&END_KEY, DocumentScroll::End),
        (&FIRST_ITEM, DocumentScroll::Start),
        (&LAST_ITEM, DocumentScroll::End),
    ];
    motions
        .into_iter()
        .find(|(key, _)| press.is(*key))
        .map(|(_, motion)| motion)
}

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
