//! The prompts, built on C15's [`StrataDialog`].

use std::rc::Rc;

use rune::{El, KeyEvent, LiveRegion, TextEdit, Tx, col, row, text, text_input};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, SegmentedControl, State,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::labels::MountLabels;
use super::request::{
    Choice, MountResponse, ProcessesRequest, PromptField, QuestionRequest, Remember, Secret,
};
use super::state::{MountPromptState, PasswordPrompt, Prompt};
use crate::contract::Point;
use crate::dialog::{Focus, MESSAGE_WIDTH, StrataDialog, Tone};
use crate::filter_bar::field::{self, Handlers, Spec};

/// The password prompt's width (Strata's `.action-dialog.wide`).
pub const PASSWORD_WIDTH: f32 = 520.0;

type Get<S, T> = Rc<dyn for<'a> Fn(&'a S) -> &'a MountPromptState<T>>;
type GetMut<S, T> = Rc<dyn for<'a> Fn(&'a mut S) -> &'a mut MountPromptState<T>>;
type Answer<S, T> = Rc<dyn Fn(&mut S, T, MountResponse)>;
type Respond<S> = Rc<dyn Fn(&mut S, MountResponse)>;
type EndProcess<S> = Rc<dyn Fn(&mut S, u32)>;
type FieldMenu<S> = Rc<dyn Fn(&mut S, PromptField, Point, &mut Tx)>;

/// The mount prompt the state holds, as a C15 dialog (see the [module docs](super)).
///
/// ```ignore
/// MountPrompts::new(
///     "mount",
///     |s: &App| &s.prompt,
///     |s: &mut App| &mut s.prompt,
///     |_s: &mut App, request: Request<MountRequest, MountResponse>, answer| {
///         request.respond(answer);
///     },
/// )
/// .labels(labels.clone())
/// .build(&look, s, &mut layer);
/// ```
///
/// Keys: the dialog is `<key>-password`, `<key>-question` or `<key>-processes`, with C15's
/// parts after it (`<key>-password-dialog`, `-primary`, `-secondary`, `-close`). The password
/// prompt adds `-user`, `-domain`, `-secret`, `-reveal`, `-connect-as`, `-remember` and
/// `-error-<n>`; a question's or process list's other choices are `-choice-<index>`; the list
/// is `-processes`, its rows `-process-<pid>` and their buttons `-end-<pid>`.
pub struct MountPrompts<S, T> {
    key: String,
    get: Get<S, T>,
    get_mut: GetMut<S, T>,
    on_answer: Answer<S, T>,
    labels: Rc<MountLabels>,
    stacked: bool,
    reduced_motion: bool,
    on_end_process: Option<EndProcess<S>>,
    on_field_menu: Option<FieldMenu<S>>,
}

impl<S, T> Clone for MountPrompts<S, T> {
    fn clone(&self) -> Self {
        MountPrompts {
            key: self.key.clone(),
            get: self.get.clone(),
            get_mut: self.get_mut.clone(),
            on_answer: self.on_answer.clone(),
            labels: self.labels.clone(),
            stacked: self.stacked,
            reduced_motion: self.reduced_motion,
            on_end_process: self.on_end_process.clone(),
            on_field_menu: self.on_field_menu.clone(),
        }
    }
}

impl<S: 'static, T: 'static> MountPrompts<S, T> {
    /// The prompts keyed `key`, on the state `get` and `get_mut` reach. `on_answer` runs once
    /// per request, after its prompt closed, with the request's ticket and the answer.
    pub fn new(
        key: impl Into<String>,
        get: impl for<'a> Fn(&'a S) -> &'a MountPromptState<T> + 'static,
        get_mut: impl for<'a> Fn(&'a mut S) -> &'a mut MountPromptState<T> + 'static,
        on_answer: impl Fn(&mut S, T, MountResponse) + 'static,
    ) -> Self {
        MountPrompts {
            key: key.into(),
            get: Rc::new(get),
            get_mut: Rc::new(get_mut),
            on_answer: Rc::new(on_answer),
            labels: Rc::new(MountLabels::default()),
            stacked: false,
            reduced_motion: false,
            on_end_process: None,
            on_field_menu: None,
        }
    }

    pub fn labels(mut self, labels: MountLabels) -> Self {
        self.labels = Rc::new(labels);
        self
    }

    /// Opened over another dialog (Connect to server): see C15's `stacked`.
    pub fn stacked(mut self, stacked: bool) -> Self {
        self.stacked = stacked;
        self
    }

    /// See C15's `reduced_motion`.
    pub fn reduced_motion(mut self, reduced: bool) -> Self {
        self.reduced_motion = reduced;
        self
    }

    /// Gives each process row an End process button, which calls `f` with its pid. Ending it
    /// is the host's (GTK sent `SIGTERM`); the backend then sends the shorter list.
    pub fn on_end_process(mut self, f: impl Fn(&mut S, u32) + 'static) -> Self {
        self.on_end_process = Some(Rc::new(f));
        self
    }

    /// A right-click, the Menu key or Shift+F10 on a field, for C21's `FieldMenu` (which
    /// offers no Cut or Copy on the secret field).
    pub fn on_field_menu(
        mut self,
        f: impl Fn(&mut S, PromptField, Point, &mut Tx) + 'static,
    ) -> Self {
        self.on_field_menu = Some(Rc::new(f));
        self
    }

    /// Pushes the prompt onto `layer`, the overlays `rune_kit::root` mounts, while one is open.
    pub fn build(&self, look: &Look, s: &S, layer: &mut Vec<El>) {
        let st = (self.get)(s);
        match st.prompt() {
            None => {}
            Some(Prompt::Password(p)) => self.password(look, p, st.attempt(), layer),
            Some(Prompt::Question(q)) => self.question(look, q, layer),
            Some(Prompt::Processes(p)) => self.processes(look, p, layer),
        }
    }

    /// Closes the prompt with an answer and hands it to the host.
    fn respond(&self) -> Respond<S> {
        let (get_mut, on_answer) = (self.get_mut.clone(), self.on_answer.clone());
        Rc::new(move |s: &mut S, r: MountResponse| {
            if let Some((ticket, r)) = get_mut(s).answer(r) {
                on_answer(s, ticket, r);
            }
        })
    }

    fn abort(&self) -> impl Fn(&mut S) + 'static {
        let respond = self.respond();
        move |s: &mut S| respond(s, MountResponse::Abort)
    }

    fn password(&self, look: &Look, p: &PasswordPrompt, attempt: u64, layer: &mut Vec<El>) {
        let (l, r, f) = (&*self.labels, &p.request, &p.form);
        let kit = &look.kit;
        let dk = format!("{}-password", self.key);
        let title = match r.secret {
            Secret::Password => &l.sign_in_title,
            Secret::Passphrase => &l.passphrase_title,
        };
        let mut d = StrataDialog::new(dk.clone(), title.clone(), self.abort())
            .open(true)
            .subtitle(l.sign_in_subtitle.clone())
            .icon(IconName::Lock)
            .width(PASSWORD_WIDTH)
            .blocking(f.typed())
            .stacked(self.stacked)
            .reduced_motion(self.reduced_motion)
            .close_label(l.close.clone());
        if !r.message.trim().is_empty() {
            d = d.message(r.message.clone());
        }
        if let Some(error) = &r.error {
            d = d.child(
                text(error.clone())
                    // A new node per refusal, so a second one is heard as well.
                    .key(format!("{dk}-error-{attempt}"))
                    .font(kit.ty.ui)
                    .color(kit.c.danger.fg)
                    .wrap()
                    .live_region(LiveRegion::Polite),
            );
        }
        let off = f.anonymous && r.anonymous_allowed;
        if r.anonymous_allowed {
            let get_mut = self.get_mut.clone();
            let pick = SegmentedControl::new(
                &[l.registered.as_str(), l.anonymous.as_str()],
                Some(usize::from(f.anonymous)),
            )
            .label(l.connect_as.clone())
            .fit()
            .on_pick(move |s: &mut S, i: usize, _tx: &mut Tx| {
                if let Some(form) = get_mut(s).password_form_mut() {
                    form.anonymous = i == 1;
                }
            })
            .build(kit)
            .key(format!("{dk}-connect-as"));
            d = d.child(group(look, &l.connect_as, pick));
        }
        let mut fields = col().w_full().gap(look.space.md);
        if r.needs_user {
            fields = fields.child(self.field(
                look,
                &dk,
                PromptField::User,
                &l.username,
                &f.user,
                off,
                None,
            ));
        }
        if r.needs_domain {
            fields = fields.child(self.field(
                look,
                &dk,
                PromptField::Domain,
                &l.domain,
                &f.domain,
                off,
                None,
            ));
        }
        if r.needs_password {
            let reveal = match (r.secret, f.revealed) {
                (Secret::Password, false) => &l.show_password,
                (Secret::Password, true) => &l.hide_password,
                (Secret::Passphrase, false) => &l.show_passphrase,
                (Secret::Passphrase, true) => &l.hide_passphrase,
            };
            let secret = l.secret(r.secret);
            fields = fields.child(self.field(
                look,
                &dk,
                PromptField::Secret,
                secret,
                &f.secret,
                off,
                Some((f.revealed, reveal.as_str())),
            ));
        }
        if r.needs_user || r.needs_domain || r.needs_password {
            d = d.child(fields);
        }
        if r.saving_allowed {
            let get_mut = self.get_mut.clone();
            let mut pick = SegmentedControl::new(
                &[l.never.as_str(), l.session.as_str(), l.forever.as_str()],
                Some(f.remember.index()),
            )
            .label(l.storage.clone())
            .fit()
            .on_pick(move |s: &mut S, i: usize, _tx: &mut Tx| {
                if let Some(form) = get_mut(s).password_form_mut() {
                    form.remember = Remember::ALL.get(i).copied().unwrap_or_default();
                }
            });
            if off {
                for i in 0..Remember::ALL.len() {
                    pick = pick.segment_state(i, State::Disabled);
                }
            }
            d = d.child(group(
                look,
                &l.storage,
                pick.build(kit).key(format!("{dk}-remember")),
            ));
        }
        // Strata's `focus_first`: a field the form focused, else Connect.
        let on_field = (r.needs_user && r.default_user.is_empty()) || r.needs_password;
        let submit = {
            let (get_mut, on_answer) = (self.get_mut.clone(), self.on_answer.clone());
            move |s: &mut S| {
                if let Some((ticket, r)) = get_mut(s).submit() {
                    on_answer(s, ticket, r);
                }
            }
        };
        d.secondary(l.cancel.clone(), self.abort())
            .primary(l.connect.clone(), submit)
            .initial_focus(if on_field {
                Focus::First
            } else {
                Focus::Primary
            })
            .build(look, layer);
    }

    /// A labelled field in C18's frame: the core `text_input`, so C21's field menu can attach
    /// (rune-kit's `TextInput` takes no right-click handler, U-C21-2).
    #[expect(
        clippy::too_many_arguments,
        reason = "one call per field, all of it the field's"
    )]
    fn field(
        &self,
        look: &Look,
        dk: &str,
        which: PromptField,
        label: &str,
        edit: &TextEdit,
        disabled: bool,
        reveal: Option<(bool, &str)>,
    ) -> El {
        let kit = &look.kit;
        let part = match which {
            PromptField::User => "user",
            PromptField::Domain => "domain",
            PromptField::Secret => "secret",
        };
        let key = format!("{dk}-{part}");
        let (get_mut, after) = (self.get_mut.clone(), self.get_mut.clone());
        let menu: field::MenuFn<S> = match &self.on_field_menu {
            Some(f) => {
                let f = f.clone();
                Rc::new(move |s: &mut S, p: Point, tx: &mut Tx| f(s, which, p, tx))
            }
            None => Rc::new(|_: &mut S, _: Point, _: &mut Tx| {}),
        };
        let handlers = Handlers {
            edit: Rc::new(move |s: &mut S| get_mut(s).edit_mut(which)),
            key: Rc::new(|_: &mut S, _: &KeyEvent, _: &mut Tx| false),
            change: Rc::new(move |s: &mut S, _: &mut Tx| after(s).edited()),
            menu: menu.clone(),
        };
        let spec = Spec {
            key: &key,
            label,
            placeholder: None,
            description: None,
            invalid: false,
            font: kit.ty.ui,
        };
        let mut input = if disabled {
            // Without handlers, as rune-kit disables its fields: `El::disabled` alone leaves a
            // text input focusable and editable.
            text_input(edit, kit.ty.ui)
                .single_line()
                .key(key.clone())
                .label(label.to_owned())
                .color(kit.c.text_muted)
                .spellcheck(false)
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .disabled(true)
        } else {
            field::input(look, edit, &spec, &handlers)
        };
        if let Some((revealed, _)) = reveal {
            input = input.revealed(revealed);
        }
        let mut inner = field::frame_row(look, format!("{key}-frame")).child(input);
        if let Some((revealed, name)) = reveal {
            let get_mut = self.get_mut.clone();
            inner = inner.child(
                IconButton::new(look.icon(if revealed {
                    IconName::EyeOff
                } else {
                    IconName::Eye
                }))
                .variant(IconButtonVariant::GhostMuted)
                .control_size(ControlSize::Small)
                .label(name.to_owned())
                .disabled(disabled)
                .on_press(move |s: &mut S| {
                    if let Some(form) = get_mut(s).password_form_mut() {
                        form.revealed = !form.revealed;
                    }
                })
                .build(kit)
                // The field stays the Tab stop, as GTK's peek icon is no stop of its own.
                .focusable(false)
                .key(format!("{dk}-reveal")),
            );
        }
        let mut frame = field::frame(look, inner, edit.focused && !disabled, false, &menu);
        if disabled {
            frame = frame.layer_opacity(0.5);
        }
        group(look, label, frame)
    }

    fn question(&self, look: &Look, q: &QuestionRequest, layer: &mut Vec<El>) {
        let l = &*self.labels;
        let dk = format!("{}-question", self.key);
        let mut d = if q.host_key {
            StrataDialog::new(dk, l.host_key_title.clone(), self.abort())
                .subtitle(l.host_key_subtitle.clone())
                .icon(IconName::Warning)
                .tone(Tone::Danger)
                .message(q.message.clone())
                .message(l.host_key_warning.clone())
        } else {
            let (title, rest) = split(&q.message);
            let d = StrataDialog::new(dk, title, self.abort()).icon(IconName::Help);
            match rest {
                Some(rest) => d.message(rest),
                None => d,
            }
        };
        d = d.width(MESSAGE_WIDTH);
        self.choices(d, &q.choices).build(look, layer);
    }

    fn processes(&self, look: &Look, p: &ProcessesRequest, layer: &mut Vec<El>) {
        let (l, kit, space) = (&*self.labels, &look.kit, look.space);
        let dk = format!("{}-processes", self.key);
        let (title, rest) = split(&p.message);
        let mut d = StrataDialog::new(dk.clone(), title, self.abort())
            .icon(IconName::Warning)
            .tone(Tone::Danger)
            .width(MESSAGE_WIDTH);
        if let Some(rest) = rest {
            d = d.message(rest);
        }
        let n = p.processes.len();
        let rows = p.processes.iter().enumerate().map(|(i, proc)| {
            let detail = match &proc.command {
                Some(cmd) => format!("{} {} · {cmd}", l.pid, proc.pid),
                None => format!("{} {}", l.pid, proc.pid),
            };
            let mut item = row()
                // Keyed by pid, so a shorter list from the backend keeps each row's focus.
                .key(format!("{dk}-process-{}", proc.pid))
                .role("listitem")
                .label(proc.name.clone())
                .description(detail.clone())
                .position_in_set(i + 1, n)
                .w_full()
                .items_center()
                .gap(space.md)
                .px(space.md)
                .py(space.sm)
                .radius(kit.radius.md)
                .fill(kit.c.layer_02)
                .child(
                    col()
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .child(
                            text(proc.name.clone())
                                .font(kit.ty.ui)
                                .color(kit.c.text_base)
                                .truncate()
                                .a11y_hidden(),
                        )
                        .child(
                            text(detail)
                                .font(kit.ty.small)
                                .color(kit.c.text_muted)
                                .truncate()
                                .a11y_hidden(),
                        ),
                );
            if let Some(end) = &self.on_end_process {
                let (end, pid) = (end.clone(), proc.pid);
                item = item.child(
                    Button::new(l.end_process.clone())
                        .variant(ButtonVariant::Neutral)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut S| end(s, pid))
                        .build(kit)
                        .key(format!("{dk}-end-{}", proc.pid)),
                );
            }
            item
        });
        d = d.child(
            col()
                .key(format!("{dk}-list"))
                .role("list")
                .label(l.processes.clone())
                .w_full()
                .gap(space.xs)
                .children(rows),
        );
        self.choices(d, &p.choices).build(look, layer);
    }

    /// A question's or a process list's footer: the first accepting choice as the primary, the
    /// first rejecting one as the secondary, the rest before them, rejecting ones first (see
    /// the module docs for how that differs from Strata's order with three or more choices).
    /// Each answers with its index in the request. No choice is the default
    /// button, so Return presses only the focused one; focus starts on the secondary, the
    /// declining choice, as Strata's host-key dialog does, or on close when none declines.
    fn choices(&self, d: StrataDialog<S>, choices: &[Choice]) -> StrataDialog<S> {
        let l = &*self.labels;
        let plan = footer(choices);
        let choose = |i: usize| {
            let respond = self.respond();
            move |s: &mut S| respond(s, MountResponse::Choice(i))
        };
        let mut d = d
            .open(true)
            .blocking(true)
            .stacked(self.stacked)
            .reduced_motion(self.reduced_motion)
            .close_label(l.close.clone())
            .default_button(false);
        for &i in &plan.actions {
            d = d.action(&format!("choice-{i}"), choices[i].label.clone(), choose(i));
        }
        if let Some(i) = plan.secondary {
            d = d.secondary(choices[i].label.clone(), choose(i));
        }
        if let Some(i) = plan.primary {
            d = d.primary(choices[i].label.clone(), choose(i));
        }
        // With no choice marked declining, every one commits: focus starts on close, which
        // aborts, as Strata's `focus_button(rejection.unwrap_or(&layout.close))`.
        d.initial_focus(match plan.secondary {
            Some(_) => Focus::Secondary,
            None => Focus::Close,
        })
    }
}

/// Where each choice sits in the footer, by its index in the request.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Footer {
    pub primary: Option<usize>,
    pub secondary: Option<usize>,
    pub actions: Vec<usize>,
}

pub(crate) fn footer(choices: &[Choice]) -> Footer {
    let primary = choices.iter().position(|c| !c.rejects);
    let secondary = choices.iter().position(|c| c.rejects);
    let mut actions: Vec<usize> = (0..choices.len())
        .filter(|&i| Some(i) != primary && Some(i) != secondary)
        .collect();
    actions.sort_by_key(|&i| !choices[i].rejects);
    Footer {
        primary,
        secondary,
        actions,
    }
}

/// GTK's split of a backend message: its first line is the title, the rest the body.
pub(crate) fn split(message: &str) -> (String, Option<String>) {
    let message = message.trim();
    match message.split_once('\n') {
        Some((title, rest)) if !rest.trim().is_empty() => {
            (title.trim().to_owned(), Some(rest.trim().to_owned()))
        }
        Some((title, _)) => (title.trim().to_owned(), None),
        None => (message.to_owned(), None),
    }
}

/// A control under its visible label. The control carries the name, so the label is hidden.
fn group(look: &Look, label: &str, control: El) -> El {
    col()
        .w_full()
        .gap(look.space.xs)
        .child(
            text(label.to_owned())
                .font(look.kit.ty.small)
                .color(look.kit.c.text_muted)
                .a11y_hidden(),
        )
        .child(control)
}
