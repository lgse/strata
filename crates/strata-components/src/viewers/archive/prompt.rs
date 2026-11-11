//! The password prompt an encrypted archive shows instead of its tree (Strata's
//! `render_archive_password_prompt`): a lock, "Password-protected archive", a line of detail,
//! a password field with an Unlock button, and the last attempt's error.
//!
//! The host checks the password (in Strata's sandbox) and answers with a tree or with
//! [`PromptState::set_error`], which empties the field, as Strata's prompt comes back empty.
//!
//! **The error is heard** as it appears: it is a polite live region, and the field is marked
//! invalid. It isn't the field's accessible description: rune-kit's `TextInput` gives its editor
//! no description (`name_field(.., None, ..)` at the pinned rev), and `TextField`, which can,
//! has no `on_submit` for Enter to unlock. (rune-kit draft R-C14b-3 in `viewers`.) Each error
//! is a new node (`<key>-error-<n>`), so a second wrong password is heard too.
//!
//! Whether the field takes focus when the prompt shows is the host's: with the preview closed,
//! Space on the archive focuses it, and Right does in List and Columns
//! ([`PromptState::focus`]); moving the cursor onto the archive leaves it unfocused.

use std::rc::Rc;

use rune::{El, LiveRegion, TextEdit, Tx, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, TextInput};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

/// The prompt's field and its last error.
#[derive(Debug)]
pub struct PromptState {
    pub edit: TextEdit,
    error: Option<String>,
    /// How many errors have been set: the error node's key, so each one is a new live node.
    attempt: u64,
}

impl Default for PromptState {
    fn default() -> Self {
        PromptState {
            edit: TextEdit::new("").secure(),
            error: None,
            attempt: 0,
        }
    }
}

impl PromptState {
    /// Asks for keyboard focus on the field, once.
    pub fn focus(&mut self) {
        self.edit.focused = true;
    }

    /// "The password is incorrect.", or none. An error empties the field, and each one is
    /// announced, even with the words of the last.
    pub fn set_error(&mut self, error: Option<String>) {
        if error.is_some() {
            self.edit.clear();
            self.attempt += 1;
        }
        self.error = error;
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// The words the prompt shows.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PromptLabels {
    pub heading: String,
    pub detail: String,
    pub placeholder: String,
    pub unlock: String,
}

impl Default for PromptLabels {
    fn default() -> Self {
        PromptLabels {
            heading: "Password-protected archive".into(),
            detail: "Enter the password to preview the archive contents".into(),
            placeholder: "Password".into(),
            unlock: "Unlock".into(),
        }
    }
}

type UnlockFn<S> = Rc<dyn Fn(&mut S, String, &mut Tx)>;

/// The prompt: `ArchivePrompt::new(&s.prompt, access).on_unlock(..).build(look)`.
pub struct ArchivePrompt<'a, S> {
    state: &'a PromptState,
    access: fn(&mut S) -> &mut PromptState,
    key: String,
    labels: PromptLabels,
    on_unlock: Option<UnlockFn<S>>,
}

impl<'a, S: 'static> ArchivePrompt<'a, S> {
    pub fn new(state: &'a PromptState, access: fn(&mut S) -> &mut PromptState) -> Self {
        ArchivePrompt {
            state,
            access,
            key: "archive-prompt".into(),
            labels: PromptLabels::default(),
            on_unlock: None,
        }
    }

    /// The prompt's key ("archive-prompt" unless set): its parts are `<key>-field`,
    /// `<key>-unlock` and `<key>-error-<n>`, where `n` counts the errors set, from 1.
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = key.into();
        self
    }

    pub fn labels(mut self, labels: PromptLabels) -> Self {
        self.labels = labels;
        self
    }

    /// Unlock or Enter, with the password typed. The field keeps it until the host answers.
    pub fn on_unlock(mut self, f: impl Fn(&mut S, String, &mut Tx) + 'static) -> Self {
        self.on_unlock = Some(Rc::new(f));
        self
    }

    pub fn build(self, look: &Look) -> El {
        let k = &look.kit;
        let key = self.key;
        let access = self.access;
        let unlock = move |f: &Option<UnlockFn<S>>| {
            let f = f.clone();
            move |s: &mut S, tx: &mut Tx| {
                let password = access(s).edit.text.clone();
                if let Some(f) = &f {
                    f(s, password, tx);
                }
            }
        };
        let field = TextInput::new(&self.state.edit)
            .secure()
            .invalid(self.state.error.is_some())
            .placeholder(self.labels.placeholder.clone())
            .label(self.labels.placeholder.clone())
            .width(Some(look.space.xxl * 8.0))
            .on_edit(move |s: &mut S| &mut access(s).edit)
            .on_submit(unlock(&self.on_unlock))
            .build(k)
            .key(format!("{key}-field"))
            .shrink()
            .min_w(look.space.xxl * 3.0);
        let button = Button::new(self.labels.unlock.clone())
            .variant(ButtonVariant::Submit)
            .on_press_tx(unlock(&self.on_unlock))
            .build(k)
            .key(format!("{key}-unlock"));
        let mut body = col()
            .key(key.clone())
            .w_full()
            .h_full()
            .padding(look.space.xl)
            .gap(look.space.md)
            .center()
            .child(
                icon(look.icon(IconName::Lock), 34.0)
                    .color(k.c.icon_muted)
                    .a11y_hidden(),
            )
            .child(
                text(self.labels.heading)
                    .key(format!("{key}-heading"))
                    .font(look.ty.title)
                    .color(k.c.text_base)
                    .wrap()
                    .align(rune::Align::Center),
            )
            .child(
                text(self.labels.detail)
                    .key(format!("{key}-detail"))
                    .font(k.ty.ui)
                    .color(k.c.text_muted)
                    .wrap()
                    .align(rune::Align::Center),
            )
            .child(
                row()
                    .gap(look.space.sm)
                    .center_y()
                    .max_w(look.space.xxl * 14.0)
                    .child(field)
                    .child(button),
            );
        if let Some(error) = self.state.error() {
            // AT-SPI announces a live region only when its node is added or its name changes
            // (accesskit_atspi_common), so a second error with the same words is a new node.
            body = body.child(
                text(error.to_owned())
                    .key(format!("{key}-error-{}", self.state.attempt))
                    .live_region(LiveRegion::Polite)
                    .font(k.ty.small)
                    .color(k.c.danger.fg)
                    .wrap()
                    .align(rune::Align::Center),
            );
        }
        body
    }
}
