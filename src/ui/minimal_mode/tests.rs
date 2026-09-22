// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn chord_cancel_returns_to_its_origin_keeping_the_fill() {
    let mut state = MinimalState::new();
    assert_eq!(state.visual(), None);
    assert_eq!(state.chord(), None);
    state.enter_visual(MinimalVisual::Select);
    state.enter_chord(MinimalChord::Go);
    assert_eq!(state.chord(), Some(MinimalChord::Go));
    assert_eq!(state.chord_from_visual(), Some(MinimalVisual::Select));
    state.cancel_chord();
    assert_eq!(state.visual(), Some(MinimalVisual::Select));
    state.enter_chord(MinimalChord::Copy);
    state.finish_chord(None);
    state.enter_chord(MinimalChord::Action);
    assert_eq!(state.chord(), Some(MinimalChord::Action));
    state.cancel_chord();
    assert_eq!(state.chord(), None);
    assert_eq!(state.chord(), None);
    assert_eq!(state.visual(), None);
    state.enter_visual(MinimalVisual::Unset);
    state.set_preview_owns_keys(true);
    state.reset();
    assert_eq!(state.chord(), None);
    assert_eq!(state.visual(), None);
    assert_eq!(state.applied_filter(), None);
    assert!(!state.preview_owns_keys());
}

#[test]
fn prompt_tracks_kind_and_last_find() {
    let mut state = MinimalState::new();
    assert_eq!(state.prompt(), None);
    state.enter_prompt(MinimalPrompt::FindNext);
    assert_eq!(state.prompt(), Some(MinimalPrompt::FindNext));
    state.record_find("read".to_owned(), 1);
    state.leave_prompt();
    assert_eq!(state.prompt(), None);
    assert_eq!(state.last_find(), Some(("read".to_owned(), 1)));
    state.reset();
    assert_eq!(state.prompt(), None);
    assert_eq!(state.last_find(), Some(("read".to_owned(), 1)));
}
