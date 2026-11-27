// SPDX-License-Identifier: MIT

//! The modal shell: backdrop, motion, dismissal, sizing and key routing around any panel.

use std::rc::Rc;

use rune::{Color, El, Key, KeyEvent, Mods, Spring, SpringPair, Transition, Tx, col, el};
use strata_kit::look::Look;

use super::keys::{self, Route};
use super::{BACKDROP_BLUR, BACKDROP_KEY, ENTRANCE_SCALE, MOTION_SECONDS, PANEL_RADIUS, WIDTH};

pub struct DialogShell<S> {
    key: String,
    label: String,
    description: Option<String>,
    children: Vec<El>,
    open: bool,
    blocking: bool,
    stacked: bool,
    width: f32,
    height: Option<f32>,
    margin: Option<f32>,
    fill: Option<Color>,
    panel_radius: Option<f32>,
    fade_only: bool,
    reduced_motion: bool,
    return_focus: Option<String>,
    footer: Vec<String>,
    dismiss: DismissFn<S>,
}

/// The shell's dismissal, with the change's `Tx`.
type DismissFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;

impl<S: 'static> DialogShell<S> {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        on_dismiss: impl Fn(&mut S) + 'static,
    ) -> Self {
        DialogShell {
            key: key.into(),
            label: label.into(),
            description: None,
            children: Vec::new(),
            open: false,
            blocking: false,
            stacked: false,
            width: WIDTH,
            height: None,
            margin: None,
            fill: None,
            panel_radius: None,
            fade_only: false,
            reduced_motion: false,
            return_focus: None,
            footer: Vec::new(),
            dismiss: Rc::new(move |s, _| on_dismiss(s)),
        }
    }

    pub fn on_dismiss_tx(mut self, on_dismiss: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.dismiss = Rc::new(on_dismiss);
        self
    }

    /// Shown while true. Closing plays the exit motion; the dialog is inert as it leaves.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// What assistive technology reads after the name: the subtitle.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds `child` to the panel, top to bottom.
    pub fn child(mut self, child: El) -> Self {
        self.children.push(child);
        self
    }

    pub fn blocking(mut self, blocking: bool) -> Self {
        self.blocking = blocking;
        self
    }

    pub fn stacked(mut self, stacked: bool) -> Self {
        self.stacked = stacked;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    pub fn margin(mut self, margin: f32) -> Self {
        self.margin = Some(margin);
        self
    }

    pub fn fill(mut self, color: Color) -> Self {
        self.fill = Some(color);
        self
    }

    pub fn panel_radius(mut self, radius: f32) -> Self {
        self.panel_radius = Some(radius);
        self
    }

    /// Opens and closes with a fade alone, no scale: Settings' layer.
    pub fn fade_only(mut self) -> Self {
        self.fade_only = true;
        self
    }

    pub fn reduced_motion(mut self, reduced: bool) -> Self {
        self.reduced_motion = reduced;
        self
    }

    pub fn return_focus(mut self, key: impl Into<String>) -> Self {
        self.return_focus = Some(key.into());
        self
    }

    pub fn footer_keys(mut self, keys: Vec<String>) -> Self {
        self.footer = keys;
        self
    }

    pub fn build(self, look: &Look, layer: &mut Vec<El>) {
        if !self.open {
            return;
        }
        let kit = &look.kit;
        let animate = look.animations && !self.reduced_motion;
        let (panel_motion, backdrop_motion) = motion(animate, self.fade_only);
        if !self.stacked {
            layer.push(backdrop(kit, backdrop_motion));
        }

        let layer_key = format!("{}-layer", self.key);
        let padding = self.margin.unwrap_or(look.space.lg);
        let panel = self.build_panel(look);

        layer.push(
            col()
                .key(layer_key)
                .role("none")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                // A narrow window: the panel fills its width less this margin.
                .padding(padding)
                .center()
                .transition(panel_motion)
                .child(panel),
        );
    }

    fn build_panel(self, look: &Look) -> El {
        let kit = &look.kit;
        let panel_key = format!("{}-dialog", self.key);
        let mut panel = col()
            .key(panel_key.clone())
            .role("dialog")
            .label(self.label)
            .w_full()
            .max_w(self.width)
            // Shrinks to the window's height; the body inside scrolls.
            .shrink()
            .min_h(0.0)
            .radius(
                self.panel_radius
                    .unwrap_or_else(|| look.round(PANEL_RADIUS)),
            )
            .fill(self.fill.unwrap_or(kit.c.layer_01))
            .border(1.0, look.transparency.frosted_border(kit.c.border_base))
            .children(self.children);
        if look.transparency.frosted_glass {
            panel = panel.backdrop_blur(look.transparency.backdrop_blur);
        }
        if let Some(height) = self.height {
            panel = panel.h_full().max_h(height);
        }
        if let Some(d) = self.description {
            panel = panel.description(d);
        }
        panel = apply_elevation(look, panel);
        let footer = self.footer;
        panel = panel.on_key(move |_s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let route = keys::route(ev, tx.key_repeat(), tx.focused(), &panel_key, &footer);
            match route {
                Route::Pass => false,
                Route::Consume => true,
                Route::Press => send(tx, Key::Text(" ".into()), false),
                Route::Focus(i) => {
                    tx.focus(footer[i].clone(), true);
                    true
                }
            }
        });
        if let Some(key) = self.return_focus {
            panel = panel.focus_return(key);
        }
        let dismiss = self.dismiss;
        panel.modal_tx(!self.blocking, move |s: &mut S, tx: &mut Tx| dismiss(s, tx))
    }
}

fn backdrop(kit: &rune_kit::Kit, motion: Transition) -> El {
    el().key(BACKDROP_KEY)
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(kit.c.scrim)
        .backdrop_blur(BACKDROP_BLUR)
        .transition(motion)
        .a11y_hidden()
}

fn apply_elevation(look: &Look, panel: El) -> El {
    if look.glow.on {
        panel.shadow(
            0.0,
            look.glow.radius,
            look.glow.color.alpha(look.glow.opacity),
        )
    } else {
        look.kit
            .shadow_layers(&look.kit.elevation.overlay)
            .fold(panel, |p, s| p.box_shadow(s))
    }
}

fn send(tx: &mut Tx, key: Key, shift: bool) -> bool {
    tx.send_key(KeyEvent {
        key,
        mods: Mods {
            shift,
            ..Mods::default()
        },
    });
    true
}

fn motion(animations: bool, fade_only: bool) -> (Transition, Transition) {
    if !animations {
        return (Transition::none(), Transition::none());
    }
    let spring = Spring::new(MOTION_SECONDS, 0.0);
    let springs = SpringPair::new(spring, spring);
    let panel = if fade_only {
        Transition::opacity()
    } else {
        Transition::scale(ENTRANCE_SCALE)
    };
    (
        panel.springs(springs),
        Transition::opacity().springs(springs),
    )
}
