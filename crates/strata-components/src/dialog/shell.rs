//! The modal shell: backdrop, motion, dismissal, sizing and key routing around any panel.

use std::rc::Rc;

use rune::{El, Key, KeyEvent, Mods, Spring, SpringPair, Transition, Tx, col, el};
use strata_kit::look::Look;

use super::keys::{self, Route};
use super::{BACKDROP_BLUR, BACKDROP_KEY, ENTRANCE_SCALE, MOTION_SECONDS, PANEL_RADIUS, WIDTH};

/// A modal panel over a blurred backdrop: what every dialog shares (see the module docs).
///
/// ```ignore
/// DialogShell::new("palette", "Search", |s: &mut App| s.palette = false)
///     .open(s.palette)
///     .width(640.0)
///     .child(palette_contents)
///     .build(&look, &mut layer);
/// ```
///
/// Keys: `<key>-layer` (the full-window layer the panel centres in, out of the accessibility
/// tree), `<key>-dialog` (the
/// panel: role `dialog`, named by `label`), and the shared [`BACKDROP_KEY`].
pub struct DialogShell<S> {
    key: String,
    label: String,
    description: Option<String>,
    children: Vec<El>,
    open: bool,
    blocking: bool,
    stacked: bool,
    width: f32,
    fade_only: bool,
    reduced_motion: bool,
    return_focus: Option<String>,
    footer: Vec<String>,
    dismiss: DismissFn<S>,
}

/// The shell's dismissal, with the change's `Tx`.
type DismissFn<S> = Rc<dyn Fn(&mut S, &mut Tx)>;

impl<S: 'static> DialogShell<S> {
    /// A closed shell keyed `key`, named `label` for assistive technology. `on_dismiss` runs
    /// on Escape and on a click outside the panel; it closes the dialog without confirming.
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
            fade_only: false,
            reduced_motion: false,
            return_focus: None,
            footer: Vec::new(),
            dismiss: Rc::new(move |s, _| on_dismiss(s)),
        }
    }

    /// Replaces `on_dismiss` with one that gets the dismissal's `Tx`: to raise an intent or
    /// move focus in the change that closes the dialog.
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

    /// Ignores clicks outside the panel while true: a dialog in a state that can't be
    /// cancelled (an operation in progress), or a form with something typed in it. Escape
    /// still calls `on_dismiss`.
    pub fn blocking(mut self, blocking: bool) -> Self {
        self.blocking = blocking;
        self
    }

    /// Opened over another dialog: draws no backdrop of its own, so the one beneath keeps the
    /// blur until the last dialog closes.
    pub fn stacked(mut self, stacked: bool) -> Self {
        self.stacked = stacked;
        self
    }

    /// The panel's width in points, when the window has room: a narrower window fills its
    /// width less a margin.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Opens and closes with a fade alone, no scale: Settings' layer.
    pub fn fade_only(mut self) -> Self {
        self.fade_only = true;
        self
    }

    /// Opens and closes at once while true, as with animations off: pass
    /// `cx.is_reduced_motion()`, the system's Reduce Motion as Rune sees it. Without it, Rune
    /// already shortens the scale to a 0.15 s fade under Reduce Motion; Strata shows no fade.
    pub fn reduced_motion(mut self, reduced: bool) -> Self {
        self.reduced_motion = reduced;
        self
    }

    /// On close, focus goes to the element keyed `key` (or its first focusable child) instead
    /// of the element focused before the dialog opened.
    pub fn return_focus(mut self, key: impl Into<String>) -> Self {
        self.return_focus = Some(key.into());
        self
    }

    /// The keys of the footer buttons that can take focus, in document order: Left and Right
    /// move between them.
    pub fn footer_keys(mut self, keys: Vec<String>) -> Self {
        self.footer = keys;
        self
    }

    /// Pushes the backdrop and the panel onto `layer`, the overlays `rune_kit::root` mounts,
    /// when open.
    pub fn build(self, look: &Look, layer: &mut Vec<El>) {
        if !self.open {
            return;
        }
        let kit = &look.kit;
        let animate = look.animations && !self.reduced_motion;
        let (panel_motion, backdrop_motion) = motion(animate, self.fade_only);
        if !self.stacked {
            layer.push(
                el().key(BACKDROP_KEY)
                    .absolute(0.0, 0.0)
                    .w_full()
                    .h_full()
                    .fill(kit.c.scrim)
                    .backdrop_blur(BACKDROP_BLUR)
                    .transition(backdrop_motion)
                    .a11y_hidden(),
            );
        }

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
            .radius(look.round(PANEL_RADIUS))
            .fill(kit.c.layer_01)
            .border(1.0, kit.c.border_base)
            .children(self.children);
        if let Some(d) = self.description {
            panel = panel.description(d);
        }
        panel = if look.glow.on {
            panel.shadow(
                0.0,
                look.glow.radius,
                look.glow.color.alpha(look.glow.opacity),
            )
        } else {
            kit.shadow_layers(&kit.elevation.overlay)
                .fold(panel, |p, s| p.box_shadow(s))
        };
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
        panel = panel.modal_tx(!self.blocking, move |s: &mut S, tx: &mut Tx| dismiss(s, tx));

        layer.push(
            col()
                .key(format!("{}-layer", self.key))
                // A layout box: assistive technology sees the dialog, not the full-window
                // layer around it. The key stays, for the layer's own presence motion.
                .role("none")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                // A narrow window: the panel fills its width less this margin.
                .padding(look.space.lg)
                .center()
                .transition(panel_motion)
                .child(panel),
        );
    }
}

/// Sends `key` through the window's key routing, after this handler. Rune applies it on the
/// next frame.
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

/// The panel's and the backdrop's presence motion: a 200 ms scale-and-fade (a fade for
/// Settings), or none when the user turned animations off. Rune follows the host's Reduce
/// Motion with a short fade; animations off means none at all, as in Strata.
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
