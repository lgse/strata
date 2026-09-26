// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    f64::consts::PI,
    rc::Rc,
    time::Instant,
};

use gtk::{gdk, glib, prelude::*};

use crate::ui::{media::DecodedMedia, theme::ThemeManager};

const NUM_BARS: usize = 84;
const NUM_BANDS: usize = 24;
const HORIZON_GAP: f64 = 1.2;
const BAR_GAP: f64 = 1.4;
const BASELINE_RATIO: f64 = 0.66;
const BOT_REFLECTION_RATIO: f64 = 0.28;

pub(super) struct SoundCloudWaveform {
    widget: gtk::DrawingArea,
    media: glib::WeakRef<gtk::MediaStream>,
    is_playing: Cell<bool>,
    decay_bands: RefCell<[f32; NUM_BANDS]>,
    bass_avg: Cell<f64>,
    beat_drop_pulse: Cell<f64>,
    last_drop_time: Cell<Option<Instant>>,
    tick_id: RefCell<Option<gtk::TickCallbackId>>,
    weak_self: RefCell<Option<std::rc::Weak<Self>>>,
}

impl SoundCloudWaveform {
    pub(super) fn new(media: &gtk::MediaStream) -> Rc<Self> {
        let area = gtk::DrawingArea::new();
        area.add_css_class("preview-soundcloud-waveform");
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_focusable(true);
        area.set_can_target(true);
        area.set_cursor_from_name(Some("pointer"));

        let weak_media = glib::WeakRef::new();
        weak_media.set(Some(media));

        let waveform = Rc::new(Self {
            widget: area.clone(),
            media: weak_media,
            is_playing: Cell::new(media.is_playing()),
            decay_bands: RefCell::new([0.0f32; NUM_BANDS]),
            bass_avg: Cell::new(0.0),
            beat_drop_pulse: Cell::new(0.0),
            last_drop_time: Cell::new(None),
            tick_id: RefCell::new(None),
            weak_self: RefCell::new(None),
        });
        waveform
            .weak_self
            .borrow_mut()
            .replace(Rc::downgrade(&waveform));

        // Setup draw function
        let weak_self = Rc::downgrade(&waveform);
        area.set_draw_func(move |_, ctx, width, height| {
            if let Some(wf) = weak_self.upgrade() {
                wf.draw(ctx, f64::from(width), f64::from(height));
            }
        });

        // Gesture: Click to toggle play/pause
        let click = gtk::GestureClick::new();
        let wf_for_click = waveform.clone();
        click.connect_pressed(move |gesture, _, _, _| {
            if let Some(w) = gesture.widget() {
                w.grab_focus();
            }
            if let Some(media) = wf_for_click.media.upgrade() {
                if media.is_playing() {
                    media.pause();
                } else {
                    media.play();
                }
            }
        });
        area.add_controller(click);

        waveform.update_playing_state(media.is_playing());
        waveform
    }

    pub(super) fn widget(&self) -> &gtk::DrawingArea {
        &self.widget
    }

    pub(super) fn update_playing_state(&self, playing: bool) {
        self.is_playing.set(playing);
        self.ensure_tick_callback();
    }

    fn ensure_tick_callback(&self) {
        if self.tick_id.borrow().is_some() {
            return;
        }
        let Some(weak_self) = self.weak_self.borrow().clone() else {
            return;
        };
        let id = self.widget.add_tick_callback(move |widget, _| {
            let Some(handle) = weak_self.upgrade() else {
                return glib::ControlFlow::Break;
            };
            widget.queue_draw();
            if !handle.is_playing.get() {
                let settled = handle.decay_bands.borrow().iter().all(|&x| x < 0.005);
                if settled {
                    handle.remove_tick_callback();
                    return glib::ControlFlow::Break;
                }
            }
            glib::ControlFlow::Continue
        });
        self.tick_id.replace(Some(id));
    }

    fn remove_tick_callback(&self) {
        if let Some(id) = self.tick_id.borrow_mut().take() {
            id.remove();
        }
    }

    fn draw(&self, ctx: &gtk::cairo::Context, width: f64, height: f64) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }

        let tokens = ThemeManager::shared().appearance_tokens();
        let accent_color = gdk::RGBA::parse(&tokens.accent)
            .or_else(|_| gdk::RGBA::parse(&tokens.highlight))
            .unwrap_or_else(|_| gdk::RGBA::new(1.0, 0.47, 0.0, 1.0));
        let ar = f64::from(accent_color.red());
        let ag = f64::from(accent_color.green());
        let ab = f64::from(accent_color.blue());

        let is_playing = self.is_playing.get();

        // Retrieve real live audio spectrum from DecodedMedia
        let mut live_spectrum = self
            .media
            .upgrade()
            .and_then(|m| m.downcast::<DecodedMedia>().ok())
            .map(|dm| dm.spectrum())
            .unwrap_or([0.0f32; NUM_BANDS]);

        // Fallback: if DecodedMedia spectrum is silent or 0, check /run/user/1000/omaramp/spectrum.json
        if is_playing
            && live_spectrum.iter().all(|&x| x <= 0.001)
            && let Ok(content) = std::fs::read_to_string("/run/user/1000/omaramp/spectrum.json")
            && let Some(start) = content.find("\"bands\":[")
        {
            let sub = &content[start + 9..];
            if let Some(end) = sub.find(']') {
                for (i, val_str) in sub[..end].split(',').take(NUM_BANDS).enumerate() {
                    if let Ok(v) = val_str.trim().parse::<f32>() {
                        live_spectrum[i] = v;
                    }
                }
            }
        }

        // Smooth decay interpolation matching Omaramp DSP
        let mut decay = self.decay_bands.borrow_mut();
        for (i, &band) in live_spectrum.iter().enumerate() {
            if is_playing {
                if band > decay[i] {
                    decay[i] = band * 0.6 + decay[i] * 0.4;
                } else {
                    decay[i] = (band * 0.25 + decay[i] * 0.75).max(0.0);
                }
            } else {
                decay[i] = (decay[i] * 0.88 - 0.02).max(0.0);
            }
        }

        // Beat drop kick calculation matching Omaramp updateBeatDrop()
        let sub_bass = (decay[0] + decay[1] + decay[2]) as f64 / 3.0;
        let avg = self.bass_avg.get() * 0.85 + sub_bass * 0.15;
        self.bass_avg.set(avg);
        let delta = sub_bass - avg;
        let now = Instant::now();
        let last_drop = self.last_drop_time.get();
        let elapsed_ms = last_drop.map_or(1000, |t| now.duration_since(t).as_millis());
        if is_playing && sub_bass > 0.40 && delta > 0.15 && elapsed_ms > 260 {
            self.beat_drop_pulse.set(1.0);
            self.last_drop_time.set(Some(now));
        } else {
            let next_pulse = (self.beat_drop_pulse.get() * 0.88 - 0.02).max(0.0);
            self.beat_drop_pulse.set(next_pulse);
        }
        let beat_drop = self.beat_drop_pulse.get();

        // Linearly resample 24 frequency bands into 84 ultra-thin bars
        let resampled = resample_bands_linear(&*decay, NUM_BARS);

        let margin = 6.0;
        let total_draw_w = (width - margin * 2.0).max(10.0);
        let num_bars = NUM_BARS;
        let gap = BAR_GAP;
        let bar_w = ((total_draw_w - (num_bars - 1) as f64 * gap) / num_bars as f64).max(1.2);
        let actual_w = num_bars as f64 * bar_w + (num_bars - 1) as f64 * gap;
        let start_x = margin + (total_draw_w - actual_w) / 2.0;

        let baseline_y = height * BASELINE_RATIO;
        let max_top_h = (baseline_y - 4.0).max(4.0);
        let max_bot_h = (height - baseline_y - 4.0).max(2.0);

        // Draw 84 live dancing waveform bars with full vibrant color fill
        for (i, &band_energy) in resampled.iter().enumerate().take(num_bars) {
            let bx = start_x + i as f64 * (bar_w + gap);

            // Dynamic wave envelope
            let env = ((i as f64 / num_bars as f64) * PI).sin();
            let shape_val = 0.16
                + env * 0.48
                + ((i as f64) * 0.55 + 0.2).sin() * 0.12
                + ((i as f64) * 1.3).sin() * 0.08;

            let energy = if is_playing { band_energy } else { 0.0 };
            let kick = if (3..=20).contains(&i) {
                beat_drop * 0.22
            } else {
                0.0
            };

            let top_norm = (shape_val * 0.38 + energy * 0.65 + kick).clamp(0.06, 1.0);
            let top_h = (top_norm * max_top_h).max(3.0);
            let bot_h = (top_h * BOT_REFLECTION_RATIO).min(max_bot_h).max(1.5);

            let top_y = baseline_y - top_h;
            let bot_y = baseline_y + HORIZON_GAP;
            let radius = (bar_w / 2.0).min(0.8);

            // ── Full Vibrant Top Bar Gradient ──
            let top_grad = gtk::cairo::LinearGradient::new(0.0, top_y, 0.0, baseline_y);
            top_grad.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.98);
            top_grad.add_color_stop_rgba(
                0.20,
                (ar + 0.20).min(1.0),
                (ag + 0.15).min(1.0),
                1.0,
                0.95,
            );
            top_grad.add_color_stop_rgba(1.0, ar, ag, ab, 0.88);
            let _ = ctx.set_source(&top_grad);
            draw_rounded_rect(ctx, bx, top_y, bar_w, top_h, radius);
            let _ = ctx.fill();

            // ── Full Vibrant Bottom Reflection ──
            let bot_grad = gtk::cairo::LinearGradient::new(0.0, bot_y, 0.0, bot_y + bot_h);
            bot_grad.add_color_stop_rgba(0.0, ar, ag, ab, 0.55);
            bot_grad.add_color_stop_rgba(1.0, ar, ag, ab, 0.15);
            let _ = ctx.set_source(&bot_grad);
            draw_rounded_rect(ctx, bx, bot_y, bar_w, bot_h, radius);
            let _ = ctx.fill();
        }
    }
}

fn resample_bands_linear(bands: &[f32], target_count: usize) -> Vec<f64> {
    if bands.is_empty() {
        return vec![0.0; target_count];
    }
    (0..target_count)
        .map(|i| {
            let t = (i as f64 / (target_count - 1).max(1) as f64) * (bands.len() - 1) as f64;
            let idx = t.floor() as usize;
            let frac = t - idx as f64;
            let next_idx = (idx + 1).min(bands.len() - 1);
            (bands[idx] as f64) * (1.0 - frac) + (bands[next_idx] as f64) * frac
        })
        .collect()
}

fn draw_rounded_rect(ctx: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    if h <= 0.0 || w <= 0.0 {
        return;
    }
    let r = r.min(w / 2.0).min(h / 2.0);
    ctx.new_sub_path();
    ctx.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    ctx.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    ctx.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    ctx.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    ctx.close_path();
}

#[cfg(test)]
mod tests;
