// SPDX-License-Identifier: MIT

//! The predicates only GTK can answer: screen pixels, the samples its own steps took, and
//! what the accessibility registry records beside the names.

use std::time::Duration;

use gtk::prelude::*;
use serde_json::{Value, json};

use super::{
    Sample,
    backend::Gtk,
    observe::{footer_fill, theme_accent},
    protocol::{self, Check, Failure, LookupError, Predicate, Scope, Target, pixels},
    samples::grab,
    targets::{self, Aim},
};

impl Gtk {
    /// `None` for a predicate the engine should have answered itself.
    pub(super) fn check_own(
        &self,
        scope: &Scope<'_, Self>,
        predicate: &Predicate,
    ) -> Result<Option<Result<(), Failure>>, String> {
        let mismatch = |actual: Value| protocol::mismatch(predicate, actual);
        let verdict = |holds: bool, actual: Value| {
            if holds { Ok(()) } else { Err(mismatch(actual)) }
        };
        let locate = |target: &Target| match scope.locate_now(target, None, Aim::Point) {
            Ok(located) => Ok(Some(located)),
            Err(LookupError::Missing(_)) => Ok(None),
            Err(LookupError::Broken(error)) => Err(error),
        };
        Ok(Some(match &predicate.check {
            Check::FooterFill(fill) => match footer_fill(scope, fill)? {
                Ok(()) => Ok(()),
                Err(actual) => Err(mismatch(actual)),
            },
            Check::Repainted(repainted) => {
                let Some(Sample::Pixels(before)) =
                    self.samples.borrow().get(&repainted.since).cloned()
                else {
                    return Err(format!("no capture was kept as {:?}", repainted.since));
                };
                let width = i64::try_from(before.width).map_err(|error| error.to_string())?;
                let height = i64::try_from(before.height).map_err(|error| error.to_string())?;
                let after = grab(self.injector()?, [before.x, before.y, width, height])?;
                let pointer = (before.x + width / 2, before.y + height / 2);
                verdict(
                    pixels::differs_beside_pointer(&before, &after, pointer),
                    json!("no pixel changed beside the pointer"),
                )
            }
            Check::ContrastBelow(below) => {
                let (_, of) = self.text_sample(&below.of)?;
                let (_, to) = self.text_sample(&below.to)?;
                let share = below.share.as_f64().unwrap_or_default();
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "pixel sums stay far below 2^52, as Python's float arithmetic assumes"
                )]
                let holds = (of as f64) < to as f64 * share;
                verdict(holds, json!({ "of": of, "to": to }))
            }
            Check::Elapsed(elapsed) => match self.samples.borrow().get(&elapsed.since) {
                Some(Sample::Instant(since)) => {
                    let passed = since.elapsed();
                    verdict(
                        passed >= Duration::from_millis(elapsed.ms),
                        json!(passed.as_millis()),
                    )
                }
                _ => return Err(format!("no instant was kept as {:?}", elapsed.since)),
            },
            Check::DiagnosticsMention(words) => {
                let words: Vec<String> = words.iter().map(|word| word.to_lowercase()).collect();
                let mentioned = self.windows().iter().any(|window| {
                    targets::presented(window.upcast_ref())
                        .iter()
                        .any(|widget| {
                            let name = targets::accessible_name(widget).to_lowercase();
                            let description = crate::ui::accessibility_qa::naming(widget)
                                .and_then(|naming| naming.description)
                                .unwrap_or_default()
                                .to_lowercase();
                            words
                                .iter()
                                .any(|word| name.contains(word) || description.contains(word))
                        })
                });
                verdict(mentioned, json!(false))
            }
            Check::Pixel(pixel) => match locate(&pixel.target)? {
                None => Err(mismatch(json!("not on screen"))),
                Some(located) => {
                    let [x, y, width, height] = located.rect.pixels();
                    let (x, y) = (x + width.div_euclid(2), y + height.div_euclid(2));
                    let colour = grab(self.injector()?, [x, y, 1, 1])?.pixels[0];
                    verdict(colour == pixel.rgb, json!(colour))
                }
            },
            Check::AccentPixels(accent) => {
                let wanted = theme_accent(self)?;
                match locate(&accent.target)? {
                    None => Err(mismatch(json!("not on screen"))),
                    Some(located) => {
                        let [x, y, width, height] = located.rect.pixels();
                        let inset = i64::from(accent.inset);
                        let capture = grab(
                            self.injector()?,
                            [x + inset, y + inset, width - 2 * inset, height - 2 * inset],
                        )?;
                        let count = capture
                            .pixels
                            .iter()
                            .filter(|colour| **colour == wanted)
                            .count();
                        verdict(
                            count > accent.more_than,
                            json!({ "accent": wanted, "count": count }),
                        )
                    }
                }
            }
            _ => return Ok(None),
        }))
    }
}
