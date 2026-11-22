// SPDX-License-Identifier: MIT

//! Selectable GPU-accelerated visual FX overlay for entering 10xer mode.
//!
//! Renders as a completely pointer-transparent, non-focusable, accessibility-hidden window
//! overlay (`Scope::Window`, `Place::Dialogs`) so it plays seamlessly over both the file browser
//! and the Settings dialog during live auditioning, never steals focus or input, respects
//! reduced motion (`look.animations`), and unmounts cleanly to zero idle CPU when complete.

use std::time::Duration;

use rune::{El, Pixels, Tx, el};
use strata_desktop::settings::TenxerFx;

use crate::{
    App, shell,
    slots::{Frame, Part, Place, Scope, Setup},
};

mod fire;
mod horizon;
mod hud;
mod palette;
mod perimeter;
mod plasma_splash;
mod shockwave;
#[cfg(test)]
mod tests;
mod warp;

use palette::FxFrame;

/// Element key of the active FX overlay container.
pub(crate) const OVERLAY_KEY: &str = "tenxer-fx-overlay";

/// Target frame step for 60 fps visual animation (coalesced per frame by Rune).
const FRAME_STEP_SECS: f64 = 1.0 / 60.0;
/// Minimum elapsed delta between redraws (120 fps ceiling; also prevents tight `turn()` loops
/// in offscreen unit tests from busy-spinning while the animation is active).
const MIN_REDRAW_STEP_SECS: f64 = 1.0 / 120.0;
const FALLBACK_WINDOW_SIZE: (f32, f32) = (1280.0, 800.0);

#[derive(Clone, Copy, Debug)]
struct ActiveFx {
    effect: TenxerFx,
    started_at: f64,
    elapsed: f64,
    duration: f64,
    seed: u32,
}

impl ActiveFx {
    fn progress(&self) -> f32 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        #[expect(clippy::cast_possible_truncation, reason = "normalized 0..=1 progress")]
        let t = (self.elapsed / self.duration).clamp(0.0, 1.0) as f32;
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
            trigger_current(app, tx);
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

fn duration_secs(effect: TenxerFx) -> f64 {
    match effect {
        TenxerFx::HorizonScan => 0.78,
        TenxerFx::VelocityWarp => 0.76,
        TenxerFx::ShockwaveRing => 0.80,
        TenxerFx::PerimeterIgnite => 0.84,
        TenxerFx::TacticalHud => 0.82,
        TenxerFx::InterfaceOnFire => 1.72,
        TenxerFx::PlasmaSplash => 1.68,
        TenxerFx::Off => 0.0,
    }
}

/// Triggers the currently saved 10xer-mode visual effect (no-op if `Off` or animations disabled).
pub(crate) fn trigger_current(app: &mut App, tx: &mut Tx) {
    let effect = shell::tenxer_fx(app);
    trigger(app, effect, tx);
}

/// Triggers a specific visual effect (used both on 10xer entry and live preview in Settings).
pub(crate) fn trigger(app: &mut App, effect: TenxerFx, tx: &mut Tx) {
    app.tenxer_fx.was_tenxer = shell::preferences(app).tenxer_mode;
    if !effect.is_enabled() || !app.look.animations {
        app.tenxer_fx.active = None;
        app.tenxer_fx.splash_pixels = None;
        return;
    }
    if effect == TenxerFx::PlasmaSplash {
        if app.tenxer_fx.splash_pixels.is_none() {
            app.tenxer_fx.splash_pixels = plasma_splash::create_pixels();
        }
    } else {
        app.tenxer_fx.splash_pixels = None;
    }
    let duration = duration_secs(effect);
    let seed = app.tenxer_fx.next_seed;
    app.tenxer_fx.next_seed = (seed % 65_535) + 1;
    app.tenxer_fx.active = Some(ActiveFx {
        effect,
        started_at: tx.clock(),
        elapsed: 0.0,
        duration,
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
    if elapsed >= active.duration {
        app.tenxer_fx.active = None;
        app.tenxer_fx.splash_pixels = None;
        return true;
    }
    let delta = elapsed - active.elapsed;
    if delta < MIN_REDRAW_STEP_SECS {
        let wait = (MIN_REDRAW_STEP_SECS - delta)
            .min(active.duration - elapsed)
            .max(0.001);
        tx.wake_after(Duration::from_secs_f64(wait));
        return false;
    }
    active.elapsed = elapsed;
    let remaining = (active.duration - elapsed).clamp(0.001, FRAME_STEP_SECS);
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
    let scene = render_effect(active.effect, &fx, app.tenxer_fx.splash_pixels.as_ref())?;
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

fn render_effect(effect: TenxerFx, frame: &FxFrame<'_>, splash: Option<&Pixels>) -> Option<El> {
    match effect {
        TenxerFx::HorizonScan => Some(horizon::build(frame)),
        TenxerFx::VelocityWarp => Some(warp::build(frame)),
        TenxerFx::ShockwaveRing => Some(shockwave::build(frame)),
        TenxerFx::PerimeterIgnite => Some(perimeter::build(frame)),
        TenxerFx::TacticalHud => Some(hud::build(frame)),
        TenxerFx::InterfaceOnFire => Some(fire::build(frame)),
        TenxerFx::PlasmaSplash => Some(plasma_splash::build(frame, splash)),
        TenxerFx::Off => None,
    }
}
