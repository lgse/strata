// SPDX-License-Identifier: MIT

pub mod ambient;
pub mod audio;
pub mod badges;
pub mod clock;
pub mod data;
pub mod intent;
pub mod keys;
pub mod labels;
mod state;
pub mod storyboard;
pub mod time;
pub(crate) mod timeline;
mod transport;
mod video;

#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Cx, El, Pixels, Tx, col, text};
use strata_kit::look::Look;

pub use ambient::{AmbientGlow, EdgeGrid};
pub use clock::{FakeClock, PlaybackClock, PlaybackState};
pub use data::{Art, AudioTags, Chapter, Clip, Details, MediaError, MediaKind};
pub use intent::MediaIntent;
pub use keys::{KeyScope, MediaBinding, MediaCommand, MediaKeys, Seek};
pub use labels::Labels;
pub use state::{GROW_INTERVAL, MediaState, STAND_IN_FADE};
pub use storyboard::{Sheet, Storyboard};
pub use timeline::levels::Levels;

use crate::viewers::viewport::ViewportState;

/// Where the view's handlers send intents.
pub(crate) type Emit<S> = Option<Rc<dyn Fn(&mut S, MediaIntent, &mut Tx)>>;

pub(crate) fn flush<S>(
    s: &mut S,
    access: fn(&mut S) -> &mut MediaState,
    emit: &Emit<S>,
    tx: &mut Tx,
) -> bool {
    let Some(f) = emit else { return false };
    let intents = access(s).drain_intents();
    for i in &intents {
        f(s, *i, tx);
    }
    !intents.is_empty()
}

pub(crate) fn commit_after<S>(
    s: &mut S,
    access: fn(&mut S) -> &mut MediaState,
    emit: &Emit<S>,
    tx: &mut Tx,
) {
    let pending = !access(s).outbox_is_empty();
    let sent = flush(s, access, emit, tx);
    if pending && !sent {
        // No callback: the host drains them in its poll, which this wake brings.
        tx.wake_after(std::time::Duration::ZERO);
    }
    if !sent {
        access(s).finish(tx);
    }
}

/// The error, inside the view: its title and detail.
pub(crate) fn error(st: &MediaState, key: &str, look: &Look) -> El {
    let k = &look.kit;
    let Some(e) = st.error() else {
        return col().key(format!("{key}-error"));
    };
    col()
        .key(format!("{key}-error"))
        .gap(look.space.xxs)
        .pt(look.space.xs)
        .child(
            text(e.title.clone())
                .key(format!("{key}-error-title"))
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .role("heading")
                .wrap(),
        )
        .child(
            text(e.detail.clone())
                .key(format!("{key}-error-detail"))
                .font(k.ty.small)
                .color(k.c.text_muted)
                .wrap(),
        )
}

pub struct MediaPlayer<'a, S> {
    state: &'a MediaState,
    access: fn(&mut S) -> &mut MediaState,
    viewport: fn(&mut S) -> &mut ViewportState,
    key: String,
    pixels: Option<&'a Pixels>,
    emit: Emit<S>,
}

impl<'a, S: 'static> MediaPlayer<'a, S> {
    pub fn new(
        state: &'a MediaState,
        access: fn(&mut S) -> &mut MediaState,
        viewport: fn(&mut S) -> &mut ViewportState,
    ) -> Self {
        MediaPlayer {
            state,
            access,
            viewport,
            key: "media".into(),
            pixels: None,
            emit: None,
        }
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    /// The frames the host writes in place (video and plain).
    pub fn video(mut self, pixels: &'a Pixels) -> Self {
        self.pixels = Some(pixels);
        self
    }

    pub fn on_intent(mut self, f: impl Fn(&mut S, MediaIntent, &mut Tx) + 'static) -> Self {
        self.emit = Some(Rc::new(f));
        self
    }

    pub fn build(self, look: &Look, cx: &mut Cx<S>) -> El {
        let MediaPlayer {
            state: st,
            access,
            viewport,
            key,
            pixels,
            emit,
        } = self;
        st.motion.set(look.animations);
        st.glow.set(look.glow.on);
        let e = match st.clip().kind {
            MediaKind::Video => {
                video::video(st, access, viewport, emit, &key, pixels, false, look, cx)
            }
            MediaKind::Plain | MediaKind::Animation => {
                video::video(st, access, viewport, emit, &key, pixels, true, look, cx)
            }
            MediaKind::Audio => audio::audio(st, access, emit, &key, look),
        };
        st.note_built();
        e
    }
}
