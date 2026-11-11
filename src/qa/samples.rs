// SPDX-License-Identifier: MIT

use std::time::{Duration, Instant};

use gtk::glib;
use serde_json::{Value, json};

use super::{
    DROP_SAMPLE_DELAYS, DROP_SAMPLE_DRIFT, DROP_SAMPLE_LIMIT, DROP_SAMPLE_RECOVERY, Outcome,
    Sample,
    backend::Gtk,
    input::{self, Event, Injector},
    protocol::{
        Failure, Scope,
        pixels::{self, Capture},
    },
};

impl Gtk {
    pub(super) fn measure_text(
        &self,
        scope: &Scope<'_, Self>,
        kept: &str,
    ) -> Result<Result<(f64, u64), Failure>, String> {
        let rect = scope.kept_box(kept)?;
        let capture = grab_on_screen(self.injector()?, rect.pixels())?;
        Ok(match pixels::text_position(&capture) {
            (Some(position), contrast) => Ok((position, contrast)),
            (None, _) => Err(Failure {
                predicate: json!({ "measure_text": kept }),
                actual: json!(0),
                message: Some("the source label disappeared".to_owned()),
            }),
        })
    }

    /// Grabs at the exact delays rather than polling, measuring only after all are taken.
    pub(super) async fn drop_samples(&self, scope: &Scope<'_, Self>, drop: &Drop<'_>) -> Outcome {
        let measured = (|| {
            Ok::<_, String>((
                scope.kept_box(drop.kept)?,
                self.text_sample(drop.resting)?.0,
                self.text_sample(drop.dragging)?.1,
                self.injector()?,
            ))
        })();
        let (rect, resting_position, dragging_contrast, injector) = match measured {
            Ok(measured) => measured,
            Err(error) => return Outcome::error(error),
        };
        let released = Instant::now();
        self.samples
            .borrow_mut()
            .insert(drop.name.to_owned(), Sample::Instant(released));
        let events = match release_events(injector, drop.escape) {
            Ok(events) => events,
            Err(error) => return Outcome::error(error),
        };
        for event in events {
            if let Err(error) = injector.send(event) {
                return Outcome::error(error);
            }
        }
        let mut grabs = Vec::new();
        for delay in DROP_SAMPLE_DELAYS {
            if let Some(remaining) = (released + delay).checked_duration_since(Instant::now()) {
                glib::timeout_future(remaining).await;
            }
            match grab_on_screen(injector, rect.pixels()) {
                Ok(capture) => grabs.push((released.elapsed(), capture)),
                Err(error) => return Outcome::error(error),
            }
        }
        judge_drop(&grabs, resting_position, dragging_contrast)
    }
}

/// What ends a drag: the button release, after an Escape press and release when asked.
fn release_events(injector: &Injector, escape: bool) -> Result<Vec<Event>, String> {
    let mut events = Vec::new();
    if escape {
        let (keycode, _) = input::keysym("Escape").and_then(|keysym| injector.keycode(keysym))?;
        events.push(Event::Key {
            keycode,
            pressed: true,
        });
        events.push(Event::Key {
            keycode,
            pressed: false,
        });
    }
    events.push(Event::Button {
        number: 1,
        pressed: false,
    });
    Ok(events)
}

/// The source label has to stay where it rested and regain its contrast, with every grab
/// taken in time.
fn judge_drop(
    grabs: &[(Duration, Capture)],
    resting_position: f64,
    dragging_contrast: u64,
) -> Outcome {
    let mut samples = Vec::new();
    for (elapsed, capture) in grabs {
        match pixels::text_position(capture) {
            (Some(position), contrast) => samples.push((*elapsed, position, contrast)),
            (None, _) => return drop_failure("the source label disappeared", &samples),
        }
    }
    if samples
        .iter()
        .any(|(elapsed, _, _)| *elapsed >= DROP_SAMPLE_LIMIT)
    {
        return drop_failure(&python_samples(&samples), &samples);
    }
    if samples
        .iter()
        .any(|(_, position, _)| (position - resting_position).abs() >= DROP_SAMPLE_DRIFT)
    {
        return drop_failure("source label slid out after drag completion", &samples);
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel sums stay far below 2^52, as Python's float arithmetic assumes"
    )]
    let recovered = samples.last().is_some_and(|(_, _, contrast)| {
        *contrast as f64 > dragging_contrast as f64 * DROP_SAMPLE_RECOVERY
    });
    if !recovered {
        return drop_failure("dragging opacity must recover", &samples);
    }
    Outcome::default()
}

/// The fields of a `drop_samples` step.
pub(super) struct Drop<'a> {
    pub(super) kept: &'a str,
    pub(super) resting: &'a str,
    pub(super) dragging: &'a str,
    pub(super) escape: bool,
    pub(super) name: &'a str,
}

impl Gtk {
    pub(super) fn text_sample(&self, name: &str) -> Result<(f64, u64), String> {
        match self.samples.borrow().get(name) {
            Some(Sample::Text { position, contrast }) => Ok((*position, *contrast)),
            _ => Err(format!("no text was measured as {name:?}")),
        }
    }
}

fn python_samples(samples: &[(Duration, f64, u64)]) -> String {
    let items: Vec<String> = samples
        .iter()
        .map(|(elapsed, position, contrast)| {
            format!("({:?}, ({position:?}, {contrast}))", elapsed.as_secs_f64())
        })
        .collect();
    format!("[{}]", items.join(", "))
}

fn drop_failure(message: &str, samples: &[(Duration, f64, u64)]) -> Outcome {
    let actual: Vec<Value> = samples
        .iter()
        .map(|(elapsed, position, contrast)| {
            json!({ "elapsed_ms": elapsed.as_millis(), "position": position, "contrast": contrast })
        })
        .collect();
    Outcome {
        failures: vec![Failure {
            predicate: json!({ "drop_samples": message }),
            actual: json!(actual),
            message: Some(message.to_owned()),
        }],
        error: Some(message.to_owned()),
        ..Outcome::default()
    }
}

fn grab_on_screen(injector: &Injector, rect: [i64; 4]) -> Result<Capture, String> {
    let [x, y, width, height] = rect;
    let (screen_width, screen_height) = injector.screen_size()?;
    if x < 0
        || y < 0
        || x + width > i64::from(screen_width)
        || y + height > i64::from(screen_height)
    {
        return Err(format!(
            "the box {rect:?} leaves the {screen_width}x{screen_height} screen"
        ));
    }
    grab(injector, rect)
}

pub(super) fn grab(
    injector: &Injector,
    [x, y, width, height]: [i64; 4],
) -> Result<Capture, String> {
    let size = |value: i64| usize::try_from(value).map_err(|error| format!("{value}: {error}"));
    if width <= 0 || height <= 0 {
        return Err(format!("cannot grab an empty box {width}x{height}"));
    }
    let (screen_width, screen_height) = injector.screen_size()?;
    let (left, top) = (x.max(0), y.max(0));
    let right = (x + width).min(i64::from(screen_width));
    let bottom = (y + height).min(i64::from(screen_height));
    let mut pixels = vec![[0, 0, 0]; size(width)? * size(height)?];
    if left < right && top < bottom {
        let narrow = |value: i64| i16::try_from(value).map_err(|error| format!("{value}: {error}"));
        let extent = |value: i64| u16::try_from(value).map_err(|error| format!("{value}: {error}"));
        let visible = injector.grab(
            narrow(left)?,
            narrow(top)?,
            extent(right - left)?,
            extent(bottom - top)?,
        )?;
        let visible_width = size(right - left)?;
        for (row, line) in visible.chunks(visible_width).enumerate() {
            let start = (size(top - y)? + row) * size(width)? + size(left - x)?;
            pixels[start..start + visible_width].copy_from_slice(line);
        }
    }
    Ok(Capture {
        x,
        y,
        width: size(width)?,
        height: size(height)?,
        pixels,
    })
}
