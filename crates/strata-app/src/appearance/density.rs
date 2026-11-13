// SPDX-License-Identifier: MIT

//! The Frame and Content densities, as GTK's Appearance menu sets them
//! (`append_density_section` in `src/ui/window/appearance_menu.rs`, `density_slider` in
//! `src/ui/window/density_slider.rs`):
//!
//! - **Saved** as a preset's name or `custom` with its px (`frame_density`,
//!   `browser_density` and their `*_custom_px`), by the shared rule
//!   (`strata_desktop::settings::stored_density_px` and `store_density_px`), on
//!   [`strata_kit::density`]'s scales: Frame 0 to 12 px, Compact 0 and Airy 4; Content 4 to
//!   16 px, Compact 4 and Airy 8. The look follows them (`shell::follow_look`) at start and on
//!   every change.
//! - **The sliders** (C20a's [`Slider`]), named "Frame" and "Content", their value text
//!   "Compact · 4 px", "Airy · 8 px" or "Custom · 6 px". Arrows step 1 px and Page Up and
//!   Page Down the distance between the presets (4 px), as GTK's `set_increments`. A drag
//!   lands on a preset near it, and the Compact and Airy toggles under their ticks set it,
//!   pressed while the value is there. A wheel over a slider scrolls what is behind it.
//! - **Every change is saved and shown at once,** a drag's moves included, as GTK saves each
//!   `value_changed`; a cancelled drag saves the value it started at again.
//! - **Return presses a focused Compact or Airy toggle,** as it presses a GTK button: C20a
//!   draws them, so the sliders' box turns a plain Return on one into the Space that presses
//!   it (`buttons::pressed`).

use rune::{El, KeyEvent, Tx, col};
use strata_components::controls::slider::{Range, Slider, SliderChange, SliderState, Snap};
use strata_core::i18n::tr;
use strata_desktop::settings::{DensityPresets, Preferences, store_density_px, stored_density_px};
use strata_kit::density::Scale;

use crate::{App, buttons, shell};

/// One of the two densities: its scale, where it is saved, and its slider's keys.
pub(crate) struct Density {
    scale: Scale,
    saved: fn(&Preferences) -> (&str, u8),
    to_save: fn(&mut Preferences) -> (&mut String, &mut u8),
    /// The slider's key, and its Compact and Airy toggles' (C20a's `<key>-snap-<i>`).
    pub(super) keys: [&'static str; 3],
    label: fn(&Labels) -> &String,
    state: fn(&App) -> &SliderState,
    state_mut: fn(&mut App) -> &mut SliderState,
}

pub(crate) const FRAME: Density = Density {
    scale: strata_kit::density::FRAME,
    saved: |saved| (&saved.frame_density, saved.frame_density_custom_px),
    to_save: |saved| (&mut saved.frame_density, &mut saved.frame_density_custom_px),
    keys: [
        "appearance-frame",
        "appearance-frame-snap-0",
        "appearance-frame-snap-1",
    ],
    label: |labels| &labels.frame,
    state: |app| &app.appearance.density.frame,
    state_mut: |app| &mut app.appearance.density.frame,
};

pub(crate) const CONTENT: Density = Density {
    scale: strata_kit::density::CONTENT,
    saved: |saved| (&saved.browser_density, saved.browser_density_custom_px),
    to_save: |saved| {
        (
            &mut saved.browser_density,
            &mut saved.browser_density_custom_px,
        )
    },
    keys: [
        "appearance-content",
        "appearance-content-snap-0",
        "appearance-content-snap-1",
    ],
    label: |labels| &labels.content,
    state: |app| &app.appearance.density.content,
    state_mut: |app| &mut app.appearance.density.content,
};

/// The sliders' state, which the popover sets from the saved values each time it opens.
pub(crate) struct Sliders {
    frame: SliderState,
    content: SliderState,
    /// The words, translated once: the language changes only on restart.
    labels: Labels,
}

pub(crate) struct Labels {
    frame: String,
    content: String,
    compact: String,
    airy: String,
}

impl Sliders {
    pub(crate) fn translated() -> Self {
        Self {
            frame: FRAME.slider_state(FRAME.scale.compact),
            content: CONTENT.slider_state(CONTENT.scale.compact),
            labels: Labels {
                frame: tr("Frame"),
                content: tr("Content"),
                compact: tr("Compact"),
                airy: tr("Airy"),
            },
        }
    }
}

impl Density {
    fn presets(&self) -> DensityPresets {
        DensityPresets {
            compact: self.scale.compact,
            airy: self.scale.airy,
            max: self.scale.max,
        }
    }

    /// The px `saved` holds for this density.
    pub(crate) fn saved(&self, saved: &Preferences) -> u8 {
        let (mode, custom_px) = (self.saved)(saved);
        stored_density_px(self.presets(), mode, custom_px)
    }

    fn slider_state(&self, px: u8) -> SliderState {
        let range = Range::new(0.0, f64::from(self.scale.max), 1.0)
            .page(f64::from(self.scale.airy - self.scale.compact));
        SliderState::new(f64::from(px), range)
    }

    /// The slider shows the saved value.
    pub(super) fn follow_saved(&self, app: &mut App) {
        let px = self.saved(shell::preferences(app));
        (self.state_mut)(app).set_value(f64::from(px));
    }

    /// Saves `px` when it is not what is saved, and has the look follow it.
    fn save(&self, app: &mut App, px: u8) {
        if self.saved(shell::preferences(app)) == px {
            return;
        }
        let (presets, to_save) = (self.presets(), self.to_save);
        shell::saved(app).change(|saved| {
            let (mode, custom_px) = to_save(saved);
            store_density_px(presets, px, mode, custom_px);
        });
        shell::follow_look(app);
    }

    /// GTK's value text: the preset's name or Custom, and the px.
    fn value_text(&self, px: u8) -> String {
        if px == self.scale.compact {
            rust_i18n::t!("Compact · %{px} px", px = px)
        } else if px == self.scale.airy {
            rust_i18n::t!("Airy · %{px} px", px = px)
        } else {
            rust_i18n::t!("Custom · %{px} px", px = px)
        }
        .into_owned()
    }

    fn slider(&'static self, app: &App) -> El {
        let labels = &app.appearance.density.labels;
        let px = self.scale.clamp((self.state)(app).value().round() as u8);
        Slider::new(self.keys[0], self.state, self.state_mut)
            .snaps(vec![
                Snap::labelled(f64::from(self.scale.compact), labels.compact.clone()),
                Snap::labelled(f64::from(self.scale.airy), labels.airy.clone()),
            ])
            .label((self.label)(labels).clone())
            .value_text(self.value_text(px))
            .on_change(move |app: &mut App, value, _: SliderChange, _: &mut Tx| {
                self.save(app, self.scale.clamp(value.round() as u8));
            })
            .build(app, &app.look)
    }
}

/// The two sliders, Frame then Content, as GTK's DENSITY section lays them out.
pub(super) fn sliders(app: &App) -> El {
    let snaps: Vec<&'static str> = [&FRAME, &CONTENT]
        .iter()
        .flat_map(|density| density.keys[1..].iter().copied())
        .collect();
    col()
        .w_full()
        .gap(app.look.space.sm)
        .px(app.look.space.sm)
        .child(FRAME.slider(app))
        .child(CONTENT.slider(app))
        .on_key(move |_: &mut App, event: &KeyEvent, tx: &mut Tx| {
            let on_snap = tx.focused().is_some_and(|key| snaps.contains(&key));
            on_snap && buttons::pressed(event, tx)
        })
}
