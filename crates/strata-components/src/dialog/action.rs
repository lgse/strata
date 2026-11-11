//! The action dialog: a header, a body and a footer of buttons, in the shell.

use std::rc::Rc;

use rune::{El, ScrollSpec, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant, Spinner};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::shell::DialogShell;
use super::text::keep_words_whole;
use super::{FOCUS_RING_ALPHA, FOCUS_RING_WIDTH, ICON_BOX, ICON_BOX_RADIUS};

/// The dialog's emphasis: its icon box, its primary button and its focus ring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Accent,
    /// A destructive confirmation, such as a permanent delete.
    Danger,
}

/// Where focus starts when the dialog opens.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    /// The first control in the body, so the first field; the first footer button when the
    /// body has none. A form opening on another field sets its `TextEdit::focused`.
    #[default]
    First,
    /// The primary button: a confirmation, so Enter confirms.
    Primary,
    /// The secondary button (Cancel), when Enter should keep things as they are.
    Secondary,
    /// The extra footer button added as `action(id, …)`; ids can come at runtime (C26's GIO
    /// choices).
    Action(String),
    /// The close button, for a dialog whose every footer button commits to something (C26's
    /// questions when no choice declines): Enter then dismisses. Without a close button
    /// ([`StrataDialog::closable`]`(false)`), as [`Focus::First`].
    Close,
}

/// The parts of a dialog keyed `<key>-<part>`, which an action id can't take.
pub const RESERVED: &[&str] = &[
    "dialog",
    "layer",
    "body",
    "title",
    "subtitle",
    "primary",
    "secondary",
    "loading",
    "close",
];

/// A body item: an element, or message text drawn with the look at build time.
enum Body {
    El(Box<El>),
    Message(String),
}

/// A button's or a dismissal's handler.
type Handler<S> = Rc<dyn Fn(&mut S)>;

struct Action<S> {
    key: String,
    label: String,
    on_press: Option<Handler<S>>,
}

impl<S: 'static> Action<S> {
    fn button(&self, look: &Look, variant: ButtonVariant, ring: rune::Color, initial: bool) -> El {
        let mut b = Button::new(self.label.clone()).variant(variant);
        b = match &self.on_press {
            Some(f) => {
                let f = f.clone();
                b.on_press(move |s: &mut S| f(s))
            }
            None => b.disabled(true),
        };
        let b = b.build(&look.kit).key(self.key.clone()).focus_ring(
            FOCUS_RING_WIDTH,
            0.0,
            ring.alpha(FOCUS_RING_ALPHA),
        );
        if initial { b.autofocus() } else { b }
    }
}

/// An action dialog (see the module docs): header, body and footer, in a [`DialogShell`].
///
/// ```ignore
/// StrataDialog::new("delete", "Permanently delete 3 items?", |s: &mut App| s.dialog = None)
///     .open(s.dialog == Some(Dialog::Delete))
///     .subtitle("3 items · 4.2 MB will be permanently deleted")
///     .icon(IconName::Trash)
///     .tone(Tone::Danger)
///     .child(file_list)
///     .secondary("Cancel", |s: &mut App| s.dialog = None)
///     .primary("Permanently delete 3 items", |s: &mut App| s.delete())
///     .initial_focus(Focus::Primary)
///     .build(&look, &mut layer);
/// ```
///
/// Keys, past the shell's: `<key>-title`, `<key>-subtitle`, `<key>-body`, `<key>-primary`,
/// `<key>-secondary`, `<key>-<action>`, `<key>-loading` and `<key>-close`.
pub struct StrataDialog<S> {
    key: String,
    title: String,
    subtitle: Option<String>,
    icon: Option<IconName>,
    tone: Tone,
    body: Vec<Body>,
    actions: Vec<Action<S>>,
    secondary: Option<Action<S>>,
    primary: Option<Action<S>>,
    initial: Focus,
    primary_enabled: bool,
    default_button: bool,
    loading: Option<String>,
    close_label: String,
    closable: bool,
    shell: DialogShell<S>,
    dismiss: Handler<S>,
}

impl<S: 'static> StrataDialog<S> {
    /// A closed dialog keyed `key` with `title`. `on_dismiss` runs on Escape, a click outside
    /// the panel and the close button: it closes without confirming.
    pub fn new(
        key: impl Into<String>,
        title: impl Into<String>,
        on_dismiss: impl Fn(&mut S) + 'static,
    ) -> Self {
        let key = key.into();
        let title = title.into();
        let dismiss: Handler<S> = Rc::new(on_dismiss);
        let shell_dismiss = dismiss.clone();
        StrataDialog {
            shell: DialogShell::new(key.clone(), title.clone(), move |s: &mut S| {
                shell_dismiss(s)
            }),
            key,
            title,
            subtitle: None,
            icon: None,
            tone: Tone::Accent,
            body: Vec::new(),
            actions: Vec::new(),
            secondary: None,
            primary: None,
            initial: Focus::First,
            primary_enabled: true,
            default_button: true,
            loading: None,
            close_label: "Close dialog".into(),
            closable: true,
            dismiss,
        }
    }

    /// Shown while true.
    pub fn open(mut self, open: bool) -> Self {
        self.shell = self.shell.open(open);
        self
    }

    /// The line under the title; assistive technology reads it as the dialog's description.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// The icon in the header's box.
    pub fn icon(mut self, name: IconName) -> Self {
        self.icon = Some(name);
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// Adds `child` to the body, top to bottom. The body scrolls when the dialog is taller
    /// than the window.
    pub fn child(mut self, child: El) -> Self {
        self.body.push(Body::El(Box::new(child)));
        self
    }

    /// Adds `children` to the body, top to bottom.
    pub fn children(mut self, children: impl IntoIterator<Item = El>) -> Self {
        self.body
            .extend(children.into_iter().map(|c| Body::El(Box::new(c))));
        self
    }

    /// Adds a paragraph of message text to the body: it wraps, keeping short Korean words
    /// whole ([`super::keep_words_whole`]).
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.body.push(Body::Message(message.into()));
        self
    }

    /// The primary button, last in the footer; Enter presses it from a single-line field or
    /// when nothing is focused. Without `enabled`, see [`StrataDialog::primary_enabled`].
    pub fn primary(
        mut self,
        label: impl Into<String>,
        on_press: impl Fn(&mut S) + 'static,
    ) -> Self {
        self.primary = Some(Action {
            key: format!("{}-primary", self.key),
            label: label.into(),
            on_press: Some(Rc::new(on_press)),
        });
        self
    }

    /// Disables the primary button while false: it can't be pressed, and Enter does nothing.
    /// Call it before or after [`StrataDialog::primary`].
    pub fn primary_enabled(mut self, enabled: bool) -> Self {
        self.primary_enabled = enabled;
        self
    }

    /// Makes the primary the dialog's default button while true, as it is unless set: Return
    /// presses it from a single-line field or with nothing focused. False for a primary that
    /// must only be pressed on purpose, such as accepting an unverified SSH host key or ending
    /// the processes that keep a volume busy (C26): Return then presses a button only when it
    /// has focus.
    pub fn default_button(mut self, default: bool) -> Self {
        self.default_button = default;
        self
    }

    /// The secondary button, before the primary: usually Cancel or Close.
    pub fn secondary(
        mut self,
        label: impl Into<String>,
        on_press: impl Fn(&mut S) + 'static,
    ) -> Self {
        self.secondary = Some(Action {
            key: format!("{}-secondary", self.key),
            label: label.into(),
            on_press: Some(Rc::new(on_press)),
        });
        self
    }

    /// Another footer button, keyed `<key>-<id>`, before the secondary. `id` can't be one of
    /// the dialog's own parts ([`RESERVED`]) or another action's.
    pub fn action(
        mut self,
        id: &str,
        label: impl Into<String>,
        on_press: impl Fn(&mut S) + 'static,
    ) -> Self {
        debug_assert!(
            !RESERVED.contains(&id),
            "action id {id:?} would collide with the dialog's own <key>-{id}"
        );
        debug_assert!(
            !self
                .actions
                .iter()
                .any(|a| a.key == format!("{}-{id}", self.key)),
            "two actions share the id {id:?}"
        );
        self.actions.push(Action {
            key: format!("{}-{id}", self.key),
            label: label.into(),
            on_press: Some(Rc::new(on_press)),
        });
        self
    }

    pub fn initial_focus(mut self, focus: Focus) -> Self {
        self.initial = focus;
        self
    }

    /// See [`DialogShell::blocking`].
    pub fn blocking(mut self, blocking: bool) -> Self {
        self.shell = self.shell.blocking(blocking);
        self
    }

    /// See [`DialogShell::stacked`].
    pub fn stacked(mut self, stacked: bool) -> Self {
        self.shell = self.shell.stacked(stacked);
        self
    }

    /// See [`DialogShell::width`].
    pub fn width(mut self, width: f32) -> Self {
        self.shell = self.shell.width(width);
        self
    }

    /// See [`DialogShell::reduced_motion`].
    pub fn reduced_motion(mut self, reduced: bool) -> Self {
        self.shell = self.shell.reduced_motion(reduced);
        self
    }

    /// See [`DialogShell::return_focus`].
    pub fn return_focus(mut self, key: impl Into<String>) -> Self {
        self.shell = self.shell.return_focus(key);
        self
    }

    /// A spinner in the header while `Some`, described by the text: "Calculating…".
    pub fn loading(mut self, description: Option<String>) -> Self {
        self.loading = description;
        self
    }

    /// The close button's accessible name, "Close dialog" until set (components carry no
    /// translations).
    pub fn close_label(mut self, label: impl Into<String>) -> Self {
        self.close_label = label.into();
        self
    }

    /// Draws no close button while false: a dialog the user leaves only through its footer,
    /// such as C16's blocking progress dialog, whose one way out is Cancel. Escape still calls
    /// `on_dismiss`. True by default.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Pushes the dialog onto `layer`, the overlays `rune_kit::root` mounts, when open.
    pub fn build(mut self, look: &Look, layer: &mut Vec<El>) {
        if !self.primary_enabled
            && let Some(p) = &mut self.primary
        {
            p.on_press = None;
        }
        let kit = &look.kit;
        let space = look.space;
        let danger = self.tone == Tone::Danger;
        let accent_ring = kit.c.text_accent;

        // The footer in document order: extra actions, the secondary, the primary.
        let mut footer: Vec<(&Action<S>, ButtonVariant, rune::Color)> = Vec::new();
        for a in self.actions.iter().chain(&self.secondary) {
            footer.push((a, ButtonVariant::Neutral, accent_ring));
        }
        let has_primary = self.primary.is_some();
        if let Some(a) = &self.primary {
            let (variant, ring) = if danger {
                (ButtonVariant::Danger, kit.c.danger.fg)
            } else {
                (ButtonVariant::Submit, accent_ring)
            };
            footer.push((a, variant, ring));
        }
        // Only buttons that can take focus count for Tab and the footer's arrows.
        let focusable: Vec<usize> = (0..footer.len())
            .filter(|&i| footer[i].0.on_press.is_some())
            .collect();
        let initial_key = match &self.initial {
            Focus::First | Focus::Close => None,
            Focus::Primary => self.primary.as_ref().map(|a| a.key.clone()),
            Focus::Secondary => self.secondary.as_ref().map(|a| a.key.clone()),
            Focus::Action(id) => {
                let key = format!("{}-{id}", self.key);
                debug_assert!(
                    self.actions.iter().any(|a| a.key == key),
                    "Focus::Action({id:?}) names no action of dialog {:?}",
                    self.key
                );
                Some(key)
            }
        };
        // The initial button takes focus as the modal opens (`El::autofocus`), and Tab goes on
        // from there in document order.
        let initial = initial_key.filter(|k| focusable.iter().any(|&i| footer[i].0.key == *k));
        let footer_keys: Vec<String> = focusable.iter().map(|&i| footer[i].0.key.clone()).collect();

        let mut buttons = row()
            .w_full()
            .flex_wrap()
            .justify_end()
            .gap(space.md)
            .px(space.xl)
            .py(space.lg);
        for (i, (action, variant, ring)) in footer.iter().enumerate() {
            let starts = initial.as_deref() == Some(action.key.as_str());
            let mut b = action.button(look, *variant, *ring, starts);
            if self.default_button && has_primary && i + 1 == footer.len() {
                // Return presses it from a single-line field or with nothing focused.
                b = b.default_button();
            }
            buttons = buttons.child(b);
        }

        let mut heading = col().grow(1.0).min_w(0.0).gap(space.xxs).child(
            text(keep_words_whole(&self.title))
                .key(format!("{}-title", self.key))
                .font(look.ty.title)
                .color(kit.c.text_base)
                .wrap()
                // The dialog carries the title as its name.
                .a11y_hidden(),
        );
        if let Some(sub) = &self.subtitle {
            heading = heading.child(
                text(keep_words_whole(sub))
                    .key(format!("{}-subtitle", self.key))
                    .font(kit.ty.small)
                    .color(kit.c.text_muted)
                    .wrap()
                    .a11y_hidden(),
            );
        }
        let mut header = row()
            .w_full()
            .gap(space.lg)
            .center_y()
            .padding(space.lg)
            .px(space.xl)
            // Room for the close button pinned in the corner.
            .pr(if self.closable {
                space.xl + look.frame.control + space.md
            } else {
                space.xl
            })
            .border_sides([0.0, 0.0, 1.0, 0.0], kit.c.border_muted);
        if let Some(name) = self.icon {
            header = header.child(icon_box(look, name, danger));
        }
        header = header.child(heading);
        if let Some(desc) = &self.loading {
            header = header.child(
                el().key(format!("{}-loading", self.key))
                    .role("progressbar")
                    .label(desc.clone())
                    // The wrapper carries the name; the spinner itself says nothing more.
                    .child(
                        Spinner::new()
                            .size(look.icon_size.md)
                            .build(kit)
                            .a11y_hidden(),
                    ),
            );
        }

        let body = col()
            .key(format!("{}-body", self.key))
            .w_full()
            .shrink()
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(
                col()
                    .w_full()
                    .gap(space.lg)
                    .px(space.xl)
                    .pt(space.lg)
                    .children(self.body.into_iter().map(|b| match b {
                        Body::El(e) => *e,
                        Body::Message(m) => paragraph(look, &m),
                    })),
            );

        let dismiss = self.dismiss.clone();
        let mut close = IconButton::new(look.icon(IconName::Close))
            .variant(IconButtonVariant::Ghost)
            .label(self.close_label.clone())
            .on_press(move |s: &mut S| dismiss(s))
            .build(kit)
            .key(format!("{}-close", self.key))
            .focus_ring(FOCUS_RING_WIDTH, 0.0, accent_ring.alpha(FOCUS_RING_ALPHA))
            .pin(None, Some(space.lg), Some(space.lg), None);
        if self.initial == Focus::Close {
            close = close.autofocus();
        }

        let mut shell = self
            .shell
            .footer_keys(footer_keys)
            .child(header)
            .child(body)
            .child(buttons);
        if self.closable {
            shell = shell.child(close);
        }
        if let Some(sub) = self.subtitle {
            shell = shell.description(sub);
        }
        shell.build(look, layer);
    }
}

fn icon_box(look: &Look, name: IconName, danger: bool) -> El {
    let c = &look.kit.c;
    let (fill, border, ink) = if danger {
        (c.danger.bg, c.danger.border, c.danger.icon)
    } else {
        (c.layer_02, c.border_base, c.icon_accent)
    };
    el().size(ICON_BOX, ICON_BOX)
        .radius(look.round(ICON_BOX_RADIUS))
        .fill(fill)
        .border(1.0, border)
        .center()
        .a11y_hidden()
        .child(icon(look.icon(name), look.icon_size.lg).color(ink))
}

/// A wrapping paragraph of message text, named with the text as written.
fn paragraph(look: &Look, message: &str) -> El {
    text(keep_words_whole(message))
        .label(message.to_owned())
        .font(look.kit.ty.ui)
        .color(look.kit.c.text_base)
        .wrap()
}
