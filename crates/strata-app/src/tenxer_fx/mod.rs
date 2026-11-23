// SPDX-License-Identifier: MIT

use std::time::Duration;

use rune::{El, Pixels, Tx, el};

use crate::{
    App, shell,
    slots::{Frame, Part, Place, Scope, Setup},
};

mod palette;
mod plasma_splash;
#[cfg(test)]
mod tests;

use palette::FxFrame;

/// Element key of the active FX overlay container.
pub(crate) const OVERLAY_KEY: &str = "tenxer-fx-overlay";

/// Total playback duration of the 10xer plasma splash animation in seconds.
const DURATION_SECS: f64 = 1.68;
/// Target frame step for 60 fps visual animation (coalesced per frame by Rune).
const FRAME_STEP_SECS: f64 = 1.0 / 60.0;
const MIN_REDRAW_STEP_SECS: f64 = 1.0 / 120.0;
const FALLBACK_WINDOW_SIZE: (f32, f32) = (1280.0, 800.0);

#[derive(Clone, Copy, Debug)]
struct ActiveFx {
    started_at: f64,
    elapsed: f64,
    seed: u32,
}

impl ActiveFx {
    fn progress(&self) -> f32 {
        #[expect(clippy::cast_possible_truncation, reason = "normalized 0..=1 progress")]
        let t = (self.elapsed / DURATION_SECS).clamp(0.0, 1.0) as f32;
        t
    }
}

pub(crate) struct TenxerFxOverlay {
    active: Option<ActiveFx>,
    splash_pixels: Option<Pixels>,
    was_tenxer: bool,
    next_seed: u32,
}

impl Part for TenxerFxOverlay {
    const SCOPE: Scope = Scope::Window;
    const PLACE: Place<App> = Place::Dialogs(build);

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            active: None,
            splash_pixels: None,
            was_tenxer: setup.settings.tenxer_mode,
            next_seed: 1,
        }
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        let enabled = shell::preferences(app).tenxer_mode;
        let mut changed = false;
        if !app.tenxer_fx.was_tenxer && enabled {
            app.tenxer_fx.was_tenxer = true;
            trigger(app, tx);
            changed = true;
        } else if app.tenxer_fx.was_tenxer && !enabled {
            app.tenxer_fx.was_tenxer = false;
            changed |= cancel(app);
        }
        if !app.look.animations {
            changed |= cancel(app);
            return changed;
        }
        changed |= tick(app, clock, tx);
        changed
    }
}

/// Triggers the 10xer-mode plasma/electricity splash animation (no-op if animations are disabled).
pub(crate) fn trigger(app: &mut App, tx: &mut Tx) {
    app.tenxer_fx.was_tenxer = shell::preferences(app).tenxer_mode;
    if !app.look.animations {
        app.tenxer_fx.active = None;
        app.tenxer_fx.splash_pixels = None;
        return;
    }
    if app.tenxer_fx.splash_pixels.is_none() {
        app.tenxer_fx.splash_pixels = plasma_splash::create_pixels();
    }
    let seed = app.tenxer_fx.next_seed;
    app.tenxer_fx.next_seed = (seed % 65_535) + 1;
    app.tenxer_fx.active = Some(ActiveFx {
        started_at: tx.clock(),
        elapsed: 0.0,
        seed,
    });
    tx.wake_after(Duration::from_secs_f64(FRAME_STEP_SECS));
}

/// Cancels any active 10x FX overlay immediately.
pub(crate) fn cancel(app: &mut App) -> bool {
    app.tenxer_fx.splash_pixels = None;
    app.tenxer_fx.active.take().is_some()
}

/// Whether a 10x FX overlay animation is currently running.
#[cfg(test)]
pub(crate) fn is_active(app: &App) -> bool {
    app.tenxer_fx.active.is_some()
}

fn tick(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let Some(active) = app.tenxer_fx.active.as_mut() else {
        return false;
    };
    let elapsed = (clock - active.started_at).max(0.0);
    if elapsed >= DURATION_SECS {
        app.tenxer_fx.active = None;
        app.tenxer_fx.splash_pixels = None;
        return true;
    }
    let delta = elapsed - active.elapsed;
    if delta < MIN_REDRAW_STEP_SECS {
        let wait = (MIN_REDRAW_STEP_SECS - delta)
            .min(DURATION_SECS - elapsed)
            .max(0.001);
        tx.wake_after(Duration::from_secs_f64(wait));
        return false;
    }
    active.elapsed = elapsed;
    let remaining = (DURATION_SECS - elapsed).clamp(0.001, FRAME_STEP_SECS);
    tx.wake_after(Duration::from_secs_f64(remaining));
    true
}

fn build(app: &App, frame: &mut Frame<'_, App>) -> Option<El> {
    if !app.look.animations || frame.cx.is_reduced_motion() {
        return None;
    }
    let active = app.tenxer_fx.active?;
    let (width, height) = app
        .window_size()
        .map(|(w, h)| (w as f32, h as f32))
        .unwrap_or(FALLBACK_WINDOW_SIZE);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "animation elapsed seconds fit f32"
    )]
    let fx = FxFrame::new(&app.look, width, height, active.progress())
        .with_timing(active.elapsed as f32, active.seed);
    let scene = plasma_splash::build(&fx, app.tenxer_fx.splash_pixels.as_ref());
    let overlay = el()
        .key(OVERLAY_KEY)
        .role("none")
        .a11y_hidden()
        .inert()
        .instant_changes()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .clip_soft(0.0)
        .child(scene);
    frame.layer.push(overlay);
    None
}
