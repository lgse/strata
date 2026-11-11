use crate::look::Look;
use crate::tokens::{self, Value};

/// Each slot token that inherits from a core token, with that token.
const SEEDS: &[(&str, &str)] = &[
    ("collection.row.radius", "radius.md"),
    ("list.row.height", "row.list"),
    ("sidebar.row.height", "row.sidebar"),
    ("tabs.tab.height", "frame.bar"),
];

fn length(look: &Look, name: &str) -> f32 {
    match tokens::get(look, name) {
        Ok(Value::Length(x)) => x,
        other => panic!("{name} read as {other:?}"),
    }
}

#[test]
fn an_unset_slot_token_follows_its_source() {
    for &(slot, source) in SEEDS {
        let mut look = Look::fallback(false);
        let before = length(&look, slot);
        let raised = length(&look, source) + 5.0;
        tokens::set(&mut look, source, Value::Length(raised)).expect("set");
        assert_eq!(length(&look, slot), before + 5.0, "{slot} follows {source}");
    }
}

#[test]
fn a_set_slot_token_stops_following_its_source() {
    for &(slot, source) in SEEDS {
        let mut look = Look::fallback(false);
        tokens::set(&mut look, slot, Value::Length(50.0)).expect("set");
        tokens::set(&mut look, source, Value::Length(10.0)).expect("set");
        assert_eq!(length(&look, slot), 50.0, "{slot} keeps its own value");
    }
}

#[test]
fn slot_token_names_are_the_slot_id_a_part_and_a_property() {
    for (id, slot) in super::TOKENS {
        for token in *slot {
            let segments: Vec<_> = token.name().split('.').collect();
            assert_eq!(segments.len(), 3, "{}", token.name());
            assert_eq!(segments[0], *id, "{} is in slot {id}", token.name());
        }
    }
}

/// A selected Icons frame deepens under the pointer, from the accent's 22 % selected fill to
/// 30 % (Strata's), as its derivation said before it had a token of its own.
#[test]
fn the_selected_frames_hover_is_its_fill_deepened_by_default() {
    use super::icons::{frame_selected_fill, frame_selected_hover_fill};

    let look = Look::fallback(false);
    let (fill, hover) = (frame_selected_fill(&look), frame_selected_hover_fill(&look));
    assert_eq!(hover, look.kit.alpha(fill, fill.a * 30.0 / 22.0));
    assert!(hover.a > fill.a, "{} over {}", hover.a, fill.a);
}

/// C13 review 1: the hover bubble's styling has tokens of its own (`.preview-video-bubble`).
#[test]
fn the_media_bubble_styling_is_overridable_by_name() {
    let look = Look::fallback(false);
    for name in [
        "media.bubble.fill",
        "media.bubble.border",
        "media.bubble.radius",
        "media.bubble.pad",
        "media.bubble.cell_fill",
        "media.bubble.cell_radius",
    ] {
        assert!(tokens::get(&look, name).is_ok(), "{name}");
    }
}
