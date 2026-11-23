// SPDX-License-Identifier: MIT

use rune::{El, Tx, row};
use rune_kit::{Button, ButtonVariant, Popover};
use strata_kit::look::Look;

use super::parts::swatch_paint;
use super::{ColorPicker, Metrics};

const POPOVER_MAX_W: f32 = 320.0;

impl<S: 'static> ColorPicker<S> {
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
