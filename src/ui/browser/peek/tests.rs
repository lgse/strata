// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::browser_modes::BrowserMode;

#[test]
fn folder_peek_uses_visible_mode_bounds() {
    assert_eq!(
        peek_origin_bounds(BrowserMode::Columns),
        PeekOriginBounds::Column
    );
    assert_eq!(
        peek_origin_bounds(BrowserMode::Icons),
        PeekOriginBounds::Anchor
    );
    assert_eq!(
        peek_origin_bounds(BrowserMode::List),
        PeekOriginBounds::Anchor
    );
}

#[test]
fn folder_peek_prefers_space_to_the_right_of_its_source_column() {
    assert_eq!(
        peek_horizontal_placement(100.0, 300.0, 800.0),
        Some(PeekPlacement {
            x: 408.0,
            side: PeekSide::Right,
        })
    );
}

#[test]
fn folder_peek_uses_the_left_only_when_it_fits_outside_the_source_column() {
    assert_eq!(
        peek_horizontal_placement(300.0, 300.0, 700.0),
        Some(PeekPlacement {
            x: 36.0,
            side: PeekSide::Left,
        })
    );
    assert_eq!(peek_horizontal_placement(200.0, 300.0, 700.0), None);
}

#[test]
fn folder_peek_animation_moves_toward_its_placement_side() {
    assert_eq!(
        peek_transition(PeekSide::Left),
        gtk::RevealerTransitionType::SlideLeft
    );
    assert_eq!(
        peek_transition(PeekSide::Right),
        gtk::RevealerTransitionType::SlideRight
    );
}

#[test]
fn folder_peek_animation_is_anchored_to_the_source_side() {
    assert_eq!(
        peek_horizontal_layout(
            PeekPlacement {
                x: 408.0,
                side: PeekSide::Right,
            },
            800.0,
        ),
        (gtk::Align::Start, 408, 0)
    );
    assert_eq!(
        peek_horizontal_layout(
            PeekPlacement {
                x: 36.0,
                side: PeekSide::Left,
            },
            700.0,
        ),
        (gtk::Align::End, 0, 408)
    );
}

#[test]
fn folder_peek_accepts_an_exact_viewport_fit() {
    assert_eq!(
        peek_horizontal_placement(0.0, 300.0, 564.0),
        Some(PeekPlacement {
            x: 308.0,
            side: PeekSide::Right,
        })
    );
}
