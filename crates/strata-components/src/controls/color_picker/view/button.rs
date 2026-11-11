//! The picker behind a button, in a rune-kit Popover: what replaces GTK's `ColorDialogButton`.

use rune::{El, Tx, row};
use rune_kit::{Button, ButtonVariant, Popover};
use strata_kit::look::Look;

use super::parts::swatch_paint;
use super::{ColorPicker, Metrics};

/// rune-kit's Popover body at its widest: `max_w(320.0)` in rune-kit's `overlay.rs` (line 233
/// at the pinned rev), which it doesn't export.
const POPOVER_MAX_W: f32 = 320.0;

impl<S: 'static> ColorPicker<S> {
    /// A button showing the colour, keyed `<key>-button` and named by
    /// [`ColorPicker::label`] (the row label, as Strata labels its theme editor's buttons),
    /// with the colour's hex as its description. A press opens the picker in a popover under
    /// it, pushed onto `layer`, and focus goes to the plane.
    ///
    /// The popover applies changes as they happen. Escape (Shift+Escape too, as Rune's modal
    /// takes both) closes it and puts back the colour it opened with
    /// ([`super::ColorChange::Cancel`]); a press outside closes it and keeps the colour, as
    /// disabling the picker does. Either way focus
    /// goes back to the button, and [`ColorPicker::on_close`] hears whether the colour changed.
    pub fn button(self, s: &S, look: &Look, layer: &mut Vec<El>) -> El {
        let st = (self.get)(s);
        let k = &look.kit;
        let m = Metrics::of(look);
        let value = st.value();
        let open = st.is_open() && !self.disabled;
        let label = self.label.clone().unwrap_or_else(|| value.hex());
        let swatch = swatch_paint(look, m, value);
        let get_mut = self.get_mut;
        let w = self.wire();
        let dismiss = w.clone();
        let button = Button::new(label.clone())
            .variant(ButtonVariant::Neutral)
            .label_with(move |_, _| swatch)
            .disabled(self.disabled)
            .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                if get_mut(s).is_open() {
                    w.close(s, false, tx);
                } else {
                    get_mut(s).open();
                }
            })
            .build(k)
            .key(format!("{}-button", self.key))
            .description(value.hex());
        let pad = look.space.lg;
        let mut popover = Popover::new(format!("{}-popover", self.key))
            .trigger(button)
            .label(label)
            .padding(pad)
            .open(open);
        if open {
            // Inside the popover's widest body the picker keeps its padding.
            let width = m.width.min(POPOVER_MAX_W - 2.0 * pad);
            // Rune's modal traps Tab, focuses the plane (its first stop) and gives focus back to
            // the button on close. A press outside dismisses it; Escape never reaches it, as the
            // picker takes Escape first (draft U-C20b-1: rune-kit's Popover has no Escape or
            // initial focus of its own).
            // The popover's own dialog, named by the label, is what assistive technology
            // hears; the body adds no second one around the parts.
            let body = self.body(s, look, m, width).role("none").modal_tx(
                true,
                move |s: &mut S, tx: &mut Tx| {
                    if get_mut(s).is_open() {
                        dismiss.close(s, false, tx);
                    }
                },
            );
            popover = popover.child(body);
        }
        row().child(popover.build(k, layer))
    }
}
