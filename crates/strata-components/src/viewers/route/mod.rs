//! The route view (`preview/preview-panel/routes`): the route picture Strata's helper draws, a
//! marker that plays along it and along the elevation profile, the profile itself drawn with
//! `canvas` shapes ([`plot`]), and the details, badges and controls of the shared player layout.
//!
//! ```ignore
//! RouteView::new(&s.route, |s: &mut App| &mut s.route, |s: &mut App| &mut s.route.viewport)
//!     .key("route")
//!     .pixels(&s.route_png)
//!     .build(&s.look, cx)
//! // and in the app's poll:
//! changed |= s.route.tick(clock, tx);
//! ```
//!
//! # What it does
//!
//! - **Showing a route.** An image named "Route preview"; the file name under the position
//!   eyebrow; badges for the format, the distance and, for a timed route, the duration. With
//!   recorded heights, an "Elevation profile" image headed "Elevation" and its range.
//! - **Playback.** The play button (or the host's media key, [`RouteState::command`]) moves the
//!   marker along the route and the profile over 15 s, at a steady ground-distance pace, and
//!   stops at the end; play then starts again from the start. Seeks pause and move the marker.
//!   The control row shows the marker's distance on the left and the total on the right, and
//!   both pictures' accessible description is the marker's distance.
//! - **Hover.** The pointer moving over either picture pauses playback and moves both markers
//!   to the point it picks; where passes overlap, the pass nearest the marker's distance among
//!   lines within 2 px of the closest ([`trace::nearest`]). Only an explicit play resumes.
//! - **Units** follow the host's preference ([`RouteState::set_units`]), without resetting
//!   playback. The view has no unit selector.
//!
//! What stays in Strata: which files are routes, the parser and its limits and messages, the
//! media key bindings, and pausing when the file changes or the preview closes or hides.
//!
//! # No commit per frame
//!
//! The markers ride `Live` offsets that [`RouteState::tick`] and the hover set, so playback and
//! hover draw without re-running the view. They commit only when the text shown changes (the
//! distance, to a tenth; the height, to a metre; the play button), and during playback at most
//! every [`state::TEXT_INTERVAL`] (about 10 Hz): on a long route the tenths change faster than
//! frames. Whatever stops playback commits the exact text at once. Strata resets both images'
//! description on every frame (its known gap); here it changes with the committed text. A
//! `LiveText` label would follow every frame with no commit, but one anywhere in the window
//! turns off Rune's virtual-window fast path for the listing beside the drawer (Rune draft
//! U-C1f-6). A hover follows the pointer, not a clock, and commits each change of text.
//!
//! # Two accessors
//!
//! As the model view: C14a's viewport takes a plain `fn` accessor, so the host passes the route
//! picture's too.

mod plot;
mod state;
pub mod trace;

#[cfg(test)]
mod tests;

use rune::{Cursor, Cx, El, Pixels, Tx, col, row, text};
use rune_kit::{IconButton, IconButtonVariant};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::contract::{Dnd, DragSource};
use crate::viewers::layout::{self, Areas};
use crate::viewers::viewport::{Interaction, Viewport, ViewportState};

pub use plot::PROFILE_HEIGHT;
pub use state::{
    PLAYBACK_SECONDS, Plot, RouteCommand, RouteData, RouteLabels, RouteState, Seek, TEXT_INTERVAL,
    Units,
};
pub use trace::RouteSample;

/// The route view: `RouteView::new(&s.route, access, viewport) … .build(look, cx)`. It fills
/// its parent.
pub struct RouteView<'a, S> {
    state: &'a RouteState,
    access: fn(&mut S) -> &mut RouteState,
    viewport: fn(&mut S) -> &mut ViewportState,
    key: String,
    pixels: Option<&'a Pixels>,
    drag: Option<(Dnd<S>, DragSource<S>)>,
}

impl<'a, S: 'static> RouteView<'a, S> {
    pub fn new(
        state: &'a RouteState,
        access: fn(&mut S) -> &mut RouteState,
        viewport: fn(&mut S) -> &mut ViewportState,
    ) -> Self {
        RouteView {
            state,
            access,
            viewport,
            key: "route".into(),
            pixels: None,
            drag: None,
        }
    }

    /// The view's key ("route" unless set). Its parts are `<key>-picture`, `<key>-profile`,
    /// `<key>-play`, `<key>-elapsed`, `<key>-total`, `<key>-height`, `<key>-range`,
    /// `<key>-format`, `<key>-distance`, `<key>-duration`, `<key>-title` and `<key>-eyebrow`.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    /// The route picture.
    pub fn pixels(mut self, pixels: &'a Pixels) -> Self {
        self.pixels = Some(pixels);
        self
    }

    /// Dragging the route picture drags the file.
    pub fn drag_source(mut self, dnd: Dnd<S>, source: DragSource<S>) -> Self {
        self.drag = Some((dnd, source));
        self
    }

    pub fn build(self, look: &Look, cx: &mut Cx<S>) -> El {
        let RouteView {
            state: st,
            access,
            viewport,
            key,
            pixels,
            drag,
        } = self;
        let k = &look.kit;
        let labels = st.labels();
        let data = st.data();
        let distance = st.distance_text();
        st.note_built();

        let mut picture = Viewport::new(&st.viewport, viewport)
            .key(format!("{key}-picture"))
            .label(labels.route.clone())
            .interaction(Interaction::Fixed)
            .on_zoom(move |s: &mut S, _r, _tx| access(s).place_markers());
        let marker = plot::marker(format!("{key}-route-marker"), st.route_marker(), look);
        picture = picture.overlay(move |_map| el_fill().child(marker));
        if let Some(px) = pixels {
            picture = picture.pixels(px);
        }
        if let Some((dnd, source)) = drag {
            picture = picture.drag_source(dnd, source);
        }
        let picture = picture.build(look, cx).description(distance.clone());
        let [iw, ih] = data.image_px;
        let content = col()
            .w_full()
            .h_full()
            .cursor(Cursor::Crosshair)
            .on_pointer_move_local(move |s: &mut S, e, tx: &mut Tx| {
                if !access(s).hover(Plot::Route, [e.x, e.y]) {
                    tx.live_only();
                }
            })
            .on_hover_change(move |s: &mut S, inside, tx: &mut Tx| {
                if !inside {
                    access(s).pointer_left();
                }
                tx.live_only();
            })
            .child(picture);

        let details = layout::details_header(
            &key,
            data.position.map(layout::Position::caption),
            &data.name,
            look,
        );
        let mut badges = vec![
            layout::badge(format!("{key}-format"), data.format.clone(), look),
            layout::badge(format!("{key}-distance"), st.total_text(), look),
        ];
        if let Some(duration) = st.duration_text() {
            badges.push(layout::badge(format!("{key}-duration"), duration, look));
        }
        let badges = layout::badge_row(&key, badges, look);

        let label = |k2: String, s: String| text(s).key(k2).font(k.ty.small).color(k.c.text_muted);
        let timeline = st.range_text().map(|range| {
            let mut heading = row().w_full().gap(look.space.md).child(
                text(labels.heading.clone())
                    .key(format!("{key}-heading"))
                    .font(k.ty.small)
                    .color(k.c.text_muted),
            );
            if let Some(height) = st.height_text() {
                heading = heading.child(label(format!("{key}-height"), height));
            }
            heading = heading.child(
                col()
                    .grow(1.0)
                    .items_end()
                    .child(label(format!("{key}-range"), range)),
            );
            let profile = col()
                .key(format!("{key}-profile"))
                .role("image")
                .label(labels.profile.clone())
                .description(distance.clone())
                .w_full()
                .h(PROFILE_HEIGHT)
                .clip_soft(0.0)
                .cursor(Cursor::Crosshair)
                .on_layout_tx(move |s: &mut S, r: [f32; 4], tx: &mut Tx| {
                    access(s).set_profile_size([r[2], r[3]]);
                    tx.live_only();
                })
                .on_pointer_move_local(move |s: &mut S, e, tx: &mut Tx| {
                    if !access(s).hover(Plot::Profile, [e.x, e.y]) {
                        tx.live_only();
                    }
                })
                .on_hover_change(move |s: &mut S, inside, tx: &mut Tx| {
                    if !inside {
                        access(s).pointer_left();
                    }
                    tx.live_only();
                })
                .child(plot::profile_shape(
                    st.profile_lines(),
                    data.profile_px,
                    look,
                ))
                .child(plot::cursor_line(
                    format!("{key}-profile-cursor"),
                    st.profile_cursor(),
                    look,
                ))
                .child(plot::marker(
                    format!("{key}-profile-marker"),
                    st.profile_marker(),
                    look,
                ));
            col()
                .key(format!("{key}-elevation"))
                .w_full()
                .gap(look.space.lg)
                .pb(look.space.lg - look.space.xs)
                .child(heading)
                .child(profile)
        });

        let play = IconButton::new(look.icon(if st.playing() {
            IconName::Pause
        } else {
            IconName::Play
        }))
        .variant(IconButtonVariant::Ghost)
        .label(labels.play.clone())
        .help(labels.play_tooltip.clone())
        .disabled(!st.playable())
        .on_press_tx(move |s: &mut S, _tx: &mut Tx| access(s).toggle())
        .build(k)
        .key(format!("{key}-play"))
        .checked(st.playing());
        let controls = row()
            .key(format!("{key}-controls"))
            .w_full()
            .center_y()
            .child(
                row()
                    .grow(1.0)
                    .child(label(format!("{key}-elapsed"), distance)),
            )
            .child(play)
            .child(
                row()
                    .grow(1.0)
                    .justify_end()
                    .child(label(format!("{key}-total"), st.total_text())),
            );

        layout::section(
            key,
            Areas {
                content,
                aspect: if ih > 0.0 { iw / ih } else { 4.0 / 3.0 },
                details,
                badges: Some(badges),
                timeline,
                controls,
            },
            look,
        )
    }
}

/// A box filling the overlay, for the marker to pin in.
fn el_fill() -> El {
    rune::el().w_full().h_full()
}
