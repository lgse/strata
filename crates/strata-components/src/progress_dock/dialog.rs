//! [`ProgressDialog`]: the blocking progress dialog, on C15's [`StrataDialog`], for operations
//! that must hold the window (Strata's moves, extracts, restores, Empty Trash, undo).

use std::rc::Rc;

use rune::{Align, El, row, text};
use strata_kit::look::Look;

use super::model::{Dock, DockIntent, OpId, Placement};
use super::view::{Bar, DIALOG_BAR};
use crate::dialog::{Focus, StrataDialog};

/// What Escape does in the progress dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EscapeDoes {
    /// Cancels the operation, as Strata's code does (`install_progress_keys`) and its feature
    /// map says ("Escape in the progress dialog acts as Cancel").
    #[default]
    Cancel,
    /// Leaves the dialog alone: the host decides (docking it, say).
    Nothing,
}

/// The speed's box in the dialog (`TRANSFER_RATE_TEXT_WIDTH_CHARS`: 26 characters,
/// `src/ui/browser/progress.rs`).
pub const RATE_WIDTH: f32 = 190.0;

type Intent<S> = Rc<dyn Fn(&mut S, DockIntent)>;

/// The blocking dialog for operation `id`, shown once the op is revealed with
/// [`Placement::Dialog`], until the host ends it. A click outside leaves it open; its one way
/// out is Cancel, which it focuses first. Cancel turns into "Cancellation requested",
/// disabled, and a transfer reads "Device not responding" 8 s later.
///
/// ```ignore
/// ProgressDialog::new("progress", |s: &App| &s.dock, |s: &mut App| &mut s.dock, op)
///     .return_focus(cursor_row_key)
///     .on_intent(|s: &mut App, intent| s.ops.handle(intent))
///     .build(s, &s.look, &mut layer);
/// ```
///
/// Keys: C15's (`<key>-dialog`, `<key>-title`, `<key>-subtitle`, `<key>-primary` for Cancel),
/// plus `<key>-bar`, `<key>-track`, `<key>-fill`, `<key>-status`, `<key>-bytes`, `<key>-items`
/// and `<key>-meta`.
pub struct ProgressDialog<S> {
    key: String,
    get: fn(&S) -> &Dock,
    get_mut: fn(&mut S) -> &mut Dock,
    id: OpId,
    on_intent: Intent<S>,
    escape: EscapeDoes,
    return_focus: Option<String>,
    reduced_motion: bool,
}

impl<S: 'static> ProgressDialog<S> {
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &Dock,
        get_mut: fn(&mut S) -> &mut Dock,
        id: OpId,
    ) -> Self {
        ProgressDialog {
            key: key.into(),
            get,
            get_mut,
            id,
            on_intent: Rc::new(|_, _| {}),
            escape: EscapeDoes::default(),
            return_focus: None,
            reduced_motion: false,
        }
    }

    /// Hears Cancel ([`DockIntent::Cancel`]). C15's handlers carry no `Tx`, so this one has
    /// none either; the dialog's wake beacon brings the tick the cancel needs
    /// ([`Dock::cancel_untimed`]).
    pub fn on_intent(mut self, f: impl Fn(&mut S, DockIntent) + 'static) -> Self {
        self.on_intent = Rc::new(f);
        self
    }

    pub fn escape(mut self, does: EscapeDoes) -> Self {
        self.escape = does;
        self
    }

    /// Where focus goes when the dialog closes: Strata's file list cursor row.
    pub fn return_focus(mut self, key: impl Into<String>) -> Self {
        self.return_focus = Some(key.into());
        self
    }

    /// See [`StrataDialog::reduced_motion`].
    pub fn reduced_motion(mut self, reduced: bool) -> Self {
        self.reduced_motion = reduced;
        self
    }

    /// Pushes the dialog onto `layer` while its operation shows in it.
    pub fn build(&self, s: &S, look: &Look, layer: &mut Vec<El>) {
        let dock = (self.get)(s);
        let Some(op) = dock.op(self.id) else {
            return;
        };
        let open = op.revealed && op.spec.placement == Placement::Dialog && op.ended().is_none();
        if !open {
            return;
        }
        let k = &look.kit;
        let c = &k.c;
        let space = look.space;
        let words = dock.words();
        let shown = &op.shown;
        let key = &self.key;
        let small = k.ty.small;

        let bar = Bar {
            key: key.clone(),
            height: DIALOG_BAR,
            label: shown.title.clone(),
        }
        .progress_described(look, op, dock.animations(), self.get_mut, &{
            // The bytes show in a live label assistive technology doesn't read: the bar says
            // them, as committed.
            match &shown.bytes {
                Some(b) if !shown.description.is_empty() => format!("{b} · {}", shown.description),
                Some(b) => b.clone(),
                None => shown.description.clone(),
            }
        });

        let mut header = row().w_full().gap(space.md).items_center();
        if shown.transfer {
            header = header
                .child(
                    text(words.transferred.clone())
                        .font(small)
                        .color(c.text_muted)
                        .a11y_hidden(),
                )
                .child(rune::el().grow(1.0))
                .child(
                    text((words.bytes_of)(
                        &(words.size)(999_900_000),
                        &(words.size)(999_900_000),
                    ))
                    .key(format!("{key}-bytes"))
                    .live_text(&op.live.bytes)
                    .align(Align::End)
                    .font(small)
                    .color(c.text_base)
                    .a11y_hidden(),
                );
        }
        if !shown.transfer {
            header = header.child(rune::el().grow(1.0));
        }
        header = header.child(
            text(words.calculating_speed.clone())
                .key(format!("{key}-status"))
                .live_text(&op.live.status)
                .align(Align::End)
                .font(small)
                .color(c.text_muted)
                .a11y_hidden(),
        );
        let mut body = rune::col().w_full().gap(space.sm).child(header).child(bar);
        if let Some(items) = &shown.items_line {
            body = body.child(
                row()
                    .w_full()
                    .gap(space.md)
                    .child(
                        text(items.clone())
                            .key(format!("{key}-items"))
                            .font(small)
                            .color(c.text_base)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate_middle(),
                    )
                    .child(
                        text(" ")
                            .key(format!("{key}-meta"))
                            .live_text(&op.live.meta)
                            .align(Align::End)
                            .font(small)
                            .color(c.text_muted)
                            .w(RATE_WIDTH)
                            .a11y_hidden(),
                    ),
            );
        }

        if let Some(generation) = dock.wants_wake() {
            body = body.child(super::view::wake_beacon::<S>(key, generation));
        }

        let id = self.id;
        let get_mut = self.get_mut;
        let intent = self.on_intent.clone();
        let cancel = move |s: &mut S| {
            if get_mut(s).cancel_untimed(id) {
                intent(s, DockIntent::Cancel(id));
            }
        };
        let escape = self.escape;
        let on_escape = {
            let cancel = cancel.clone();
            move |s: &mut S| {
                if escape == EscapeDoes::Cancel {
                    cancel(s);
                }
            }
        };
        let mut d = StrataDialog::new(key.clone(), shown.title.clone(), on_escape)
            .open(true)
            .closable(false)
            .blocking(true)
            .icon(op.spec.icon)
            .child(body)
            .primary(shown.cancel_words.clone(), cancel)
            .primary_enabled(shown.cancel_enabled)
            .initial_focus(Focus::Primary)
            .reduced_motion(self.reduced_motion);
        if let Some(sub) = &shown.subtitle {
            d = d.subtitle(sub.clone());
        }
        if let Some(r) = &self.return_focus {
            d = d.return_focus(r.clone());
        }
        d.build(look, layer);
    }
}
