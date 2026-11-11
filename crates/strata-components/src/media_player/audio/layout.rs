//! Where the audio view puts its art and spectrum (Strata's
//! `src/ui/preview/audio/layout.rs`, `NowPlayingLayout`).
//!
//! A pane at least 1.12 times as wide as tall puts the art beside the rest; at 0.95 or below
//! the art goes back on top, and between the two the layout keeps what it had. Stacked, when
//! height runs short the spectrum hides first, then the art.

/// Strata's limits, in points.
pub const ART_MIN: f32 = 96.0;
pub const ART_MAX: f32 = 520.0;
pub const SPECTRUM_MIN: f32 = 44.0;
pub const SPECTRUM_MAX: f32 = 200.0;
/// The widest the panel of tags and controls grows.
pub const PANEL_MAX: f32 = 620.0;
/// The share of the free height stacked art takes, and of the width wide art takes.
const ART_SHARE: f32 = 0.62;
const WIDE_ART_SHARE: f32 = 0.5;
const ENTER_WIDE: f32 = 1.12;
const LEAVE_WIDE: f32 = 0.95;

/// What the audio view shows and how big.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plan {
    pub wide: bool,
    /// The art's side, or `None` when hidden.
    pub art: Option<f32>,
    /// The spectrum's height, or `None` when hidden.
    pub spectrum: Option<f32>,
}

fn round(x: f32) -> f32 {
    x.round()
}

/// The plan for a `width` × `height` pane. `fixed` is the height the tags, timeline and
/// controls need, `gap` the space between rows, and `was_wide` the last plan's mode.
pub fn plan(width: f32, height: f32, fixed: f32, gap: f32, was_wide: bool) -> Plan {
    if width <= 0.0 || height <= 0.0 {
        return Plan::default();
    }
    let ratio = width / height;
    let wide = if was_wide {
        ratio > LEAVE_WIDE
    } else {
        ratio >= ENTER_WIDE
    };
    if wide {
        let art = (width * WIDE_ART_SHARE).min(height).min(ART_MAX);
        let free = height - fixed - gap;
        let spectrum = (free >= SPECTRUM_MIN).then(|| round(free.min(SPECTRUM_MAX)));
        return Plan {
            wide,
            art: (art >= ART_MIN).then(|| round(art)),
            spectrum,
        };
    }
    let free = height - fixed;
    if free >= ART_MIN + SPECTRUM_MIN + 2.0 * gap {
        let art = (free * ART_SHARE).clamp(ART_MIN, ART_MAX.min(width));
        let spectrum = (free - art - 2.0 * gap).clamp(SPECTRUM_MIN, SPECTRUM_MAX);
        Plan {
            wide,
            art: Some(round(art)),
            spectrum: Some(round(spectrum)),
        }
    } else if free >= ART_MIN + gap {
        Plan {
            wide,
            art: Some(round((free - gap).min(ART_MAX).min(width))),
            spectrum: None,
        }
    } else {
        Plan {
            wide,
            art: None,
            spectrum: None,
        }
    }
}
