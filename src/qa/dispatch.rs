// SPDX-License-Identifier: MIT

//! The steps only GTK runs: revealing a control, setting a scroll bar, screen captures and
//! a clipboard on a display of its own.

use gtk::prelude::*;
use serde_json::{Value, json};

use super::{
    Outcome, Sample,
    backend::Gtk,
    clipboard::{abridged, own_clipboard, private_display, read_clipboard},
    protocol::{self, Aim, Command, Failure, Scope},
    reveal::set_scrollbar,
    samples::{Drop, grab},
};

impl Gtk {
    /// `None` for a step the engine should have run itself.
    pub(super) async fn command_own(
        &self,
        scope: &Scope<'_, Self>,
        command: &Command,
    ) -> Option<Outcome> {
        Some(match command {
            Command::RevealControl { target } => self.reveal_control(scope, target).await,
            Command::Reveal { target } => self.reveal(scope, target).await,
            Command::SetScrollbar { orientation, value } => {
                Outcome::from(set_scrollbar(&scope.window, *orientation, *value))
            }
            Command::Capture {
                target,
                inset,
                name,
            } => match scope
                .find(target, None, Aim::Point)
                .await
                .and_then(|located| {
                    let [x, y, width, height] = located.rect.pixels();
                    let inset = i64::from(*inset);
                    grab(
                        self.injector()?,
                        [x + inset, y + inset, width - 2 * inset, height - 2 * inset],
                    )
                }) {
                Ok(capture) => {
                    self.samples
                        .borrow_mut()
                        .insert(name.clone(), Sample::Pixels(capture));
                    Outcome::default()
                }
                Err(error) => Outcome::error(error),
            },
            Command::MeasureText { kept, name } => match self.measure_text(scope, kept) {
                Ok(Ok((position, contrast))) => {
                    self.samples
                        .borrow_mut()
                        .insert(name.clone(), Sample::Text { position, contrast });
                    Outcome::default()
                }
                Ok(Err(failure)) => Outcome {
                    failures: vec![failure],
                    error: Some("the label has no ink".to_owned()),
                    ..Outcome::default()
                },
                Err(error) => Outcome::error(error),
            },
            Command::DropSamples {
                kept,
                resting,
                dragging,
                escape,
                name,
            } => {
                let drop = Drop {
                    kept,
                    resting,
                    dragging,
                    escape: *escape,
                    name,
                };
                self.drop_samples(scope, &drop).await
            }
            Command::ClipboardSet(content) => match private_display() {
                Ok(display) => {
                    if let Err(error) = own_clipboard(&display, content.clone()) {
                        display.close();
                        return Some(Outcome::error(error));
                    }
                    if let Some(previous) = self.clipboard.replace(Some(display)) {
                        previous.close();
                    }
                    Outcome::default()
                }
                Err(error) => Outcome::error(error),
            },
            Command::Clipboard {
                text,
                mime,
                message,
            } => {
                let display = match self.clipboard.take().map_or_else(private_display, Ok) {
                    Ok(display) => display,
                    Err(error) => return Some(Outcome::error(error)),
                };
                let read = read_clipboard(&display, mime.as_deref()).await;
                display.close();
                let predicate = json!({ "clipboard": text, "mime": mime });
                match read {
                    Some(read) if text.matches(&read) => Outcome::default(),
                    Some(read) => Outcome {
                        failures: vec![Failure {
                            predicate,
                            actual: json!(abridged(&read)),
                            message: message.clone(),
                        }],
                        error: Some("the clipboard holds other text".to_owned()),
                        ..Outcome::default()
                    },
                    None => {
                        let failure = Failure {
                            predicate,
                            actual: Value::Null,
                            message: Some(
                                "selected table text on the private display clipboard".to_owned(),
                            ),
                        };
                        scope.timed_out(failure, protocol::DEFAULT_WAIT)
                    }
                }
            }
            _ => return None,
        })
    }
}
