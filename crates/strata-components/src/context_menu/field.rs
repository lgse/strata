// SPDX-License-Identifier: MIT

use std::rc::Rc;

use rune::{El, PointerPress, TextEdit, Tx, el};
use strata_kit::look::Look;

use super::model::{Item, Section};
use super::state::menu_bar;
use super::view::{Render, width};
use crate::contract::Point;
use crate::scroll::Bar;

/// The field menu's words, in the interface language. English by default.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldLabels {
    /// The menu's accessible name.
    pub menu: String,
    pub undo: String,
    pub redo: String,
    pub cut: String,
    pub copy: String,
    pub paste: String,
    pub delete: String,
    pub select_all: String,
}

impl Default for FieldLabels {
    fn default() -> Self {
        Self {
            menu: "Text editing".to_owned(),
            undo: "Undo".to_owned(),
            redo: "Redo".to_owned(),
            cut: "Cut".to_owned(),
            copy: "Copy".to_owned(),
            paste: "Paste".to_owned(),
            delete: "Delete".to_owned(),
            select_all: "Select All".to_owned(),
        }
    }
}

/// A field menu's command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

impl FieldCommand {
    fn id(self) -> &'static str {
        match self {
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Cut => "cut",
            Self::Copy => "copy",
            Self::Paste => "paste",
            Self::Delete => "delete",
            Self::SelectAll => "select-all",
        }
    }

    fn label(self, l: &FieldLabels) -> &str {
        match self {
            Self::Undo => &l.undo,
            Self::Redo => &l.redo,
            Self::Cut => &l.cut,
            Self::Copy => &l.copy,
            Self::Paste => &l.paste,
            Self::Delete => &l.delete,
            Self::SelectAll => &l.select_all,
        }
    }
}

/// What the clipboard held when the menu opened, as far as the menu knows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Clipboard {
    /// The read hasn't answered: Paste stays sensitive.
    #[default]
    Unknown,
    /// Text to paste.
    Text,
    /// Nothing Paste can paste: Paste is insensitive.
    Empty,
}

/// Which field menu is open, held by the host. `F` names the host's fields.
#[derive(Clone, Debug)]
pub struct FieldMenuState<F> {
    open: Option<Open<F>>,
    /// Counts opens, so a clipboard read that answers after its menu closed is dropped.
    serial: u64,
}

#[derive(Clone, Debug)]
struct Open<F> {
    field: F,
    at: Point,
    clipboard: Clipboard,
    history: (bool, bool),
    scroll_bar: Bar,
}

impl<F> Default for FieldMenuState<F> {
    fn default() -> Self {
        Self {
            open: None,
            serial: 0,
        }
    }
}

impl<F: Copy + PartialEq> FieldMenuState<F> {
    /// No menu open.
    pub fn new() -> Self {
        Self::default()
    }

    /// The field whose menu is open.
    pub fn field(&self) -> Option<F> {
        self.open.as_ref().map(|o| o.field)
    }

    /// Whether a menu is open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// What the menu knows of the clipboard.
    pub fn clipboard(&self) -> Clipboard {
        self.open
            .as_ref()
            .map_or(Clipboard::Unknown, |o| o.clipboard)
    }

    /// Closes the menu. The field keeps its text and selection.
    pub fn close(&mut self) -> Option<F> {
        let open = self.open.take()?;
        open.scroll_bar.release();
        Some(open.field)
    }

    pub fn open(&mut self, field: F, at: Point) {
        self.open_serial(field, at, (false, false));
    }

    fn open_serial(&mut self, field: F, at: Point, history: (bool, bool)) -> u64 {
        if let Some(prev) = self.open.take() {
            prev.scroll_bar.release();
        }
        self.serial += 1;
        self.open = Some(Open {
            field,
            at,
            clipboard: Clipboard::Unknown,
            history,
            scroll_bar: menu_bar(),
        });
        self.serial
    }

    pub fn clipboard_known(&mut self, has_text: bool) {
        self.clipboard_read(self.serial, has_text);
    }

    /// The clipboard read for open `serial` answered.
    fn clipboard_read(&mut self, serial: u64, has_text: bool) {
        if serial == self.serial
            && let Some(o) = &mut self.open
        {
            o.clipboard = if has_text {
                Clipboard::Text
            } else {
                Clipboard::Empty
            };
        }
    }
}

type EditMut<S, F> = fn(&mut S, F) -> Option<&mut TextEdit>;
type EditRef<S, F> = fn(&S, F) -> Option<&TextEdit>;
type Changed<S, F> = Rc<dyn Fn(&mut S, F, &mut Tx)>;
type Pasted<S, F> = Rc<dyn Fn(&mut S, F, &str, &mut Tx)>;
type Closed<S, F> = Rc<dyn Fn(&mut S, F)>;

pub struct FieldMenu<S, F> {
    key: String,
    get: fn(&S) -> &FieldMenuState<F>,
    get_mut: fn(&mut S) -> &mut FieldMenuState<F>,
    edit_mut: EditMut<S, F>,
    edit: EditRef<S, F>,
    labels: FieldLabels,
    editable: bool,
    undo: bool,
    changed: Option<Changed<S, F>>,
    pasted: Option<Pasted<S, F>>,
    closed: Option<Closed<S, F>>,
}

impl<S: 'static, F: Copy + PartialEq + 'static> FieldMenu<S, F> {
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &FieldMenuState<F>,
        get_mut: fn(&mut S) -> &mut FieldMenuState<F>,
        edit_mut: EditMut<S, F>,
        edit: EditRef<S, F>,
    ) -> Self {
        Self {
            key: key.into(),
            get,
            get_mut,
            edit_mut,
            edit,
            labels: FieldLabels::default(),
            editable: true,
            undo: false,
            changed: None,
            pasted: None,
            closed: None,
        }
    }

    /// The menu's words.
    pub fn labels(mut self, labels: FieldLabels) -> Self {
        self.labels = labels;
        self
    }

    /// A read-only field: Cut, Paste and Delete are insensitive.
    pub fn read_only(mut self) -> Self {
        self.editable = false;
        self
    }

    pub fn undo(mut self, on: bool) -> Self {
        self.undo = on;
        self
    }

    /// Hears a command change the field's text: validate it again, filter suggestions.
    pub fn on_change(mut self, f: impl Fn(&mut S, F, &mut Tx) + 'static) -> Self {
        self.changed = Some(Rc::new(f));
        self
    }

    pub fn on_paste(mut self, f: impl Fn(&mut S, F, &str, &mut Tx) + 'static) -> Self {
        self.pasted = Some(Rc::new(f));
        self
    }

    pub fn on_close(mut self, f: impl Fn(&mut S, F) + 'static) -> Self {
        self.closed = Some(Rc::new(f));
        self
    }

    pub fn check(&self, s: &mut S) -> bool {
        let Some(field) = (self.get)(s).field() else {
            return false;
        };
        if (self.edit)(s, field).is_some() {
            return false;
        }
        (self.get_mut)(s).close();
        if let Some(f) = &self.closed {
            f(s, field);
        }
        true
    }

    pub fn open(&self, s: &mut S, field: F, at: Point, tx: &mut Tx) {
        self.check(s);
        let Some(e) = (self.edit)(s, field) else {
            return;
        };
        let history = if self.undo {
            history(e)
        } else {
            (false, false)
        };
        let serial = (self.get_mut)(s).open_serial(field, at, history);
        let get_mut = self.get_mut;
        tx.read_clipboard(move |s: &mut S, content, _| {
            get_mut(s).clipboard_read(serial, !content.text().is_empty());
        });
    }

    pub fn hook(self, field: F) -> impl Fn(&mut S, PointerPress, &mut Tx) + 'static {
        move |s: &mut S, p: PointerPress, tx: &mut Tx| self.open(s, field, Point::new(p.x, p.y), tx)
    }

    pub fn build(self, s: &S, look: &Look, layer: &mut Vec<El>) -> El {
        let st = (self.get)(s);
        let Some(open) = &st.open else {
            return el().absolute(0.0, 0.0);
        };
        let Some(edit) = (self.edit)(s, open.field) else {
            return el().absolute(0.0, 0.0);
        };
        let commands = self.commands(edit, open.clipboard, open.history);
        let sections = sections(&commands, &self.labels);
        let me = Rc::new(self);
        let (select, dismiss) = (me.clone(), me.clone());
        let field = open.field;
        let scroll_bar = open.scroll_bar.clone();
        Render {
            key: &me.key,
            at: open.at,
            name: &me.labels.menu,
            header: &[],
            sections: &sections,
            open: &[],
            top_width: width(look, &[], &sections),
            keeps_text_selection: false,
            scroll_bars: vec![scroll_bar],
            look,
            on_select: Rc::new(move |s: &mut S, path: &[usize], tx: &mut Tx| {
                let command = path.first().and_then(|&i| commands.get(i)).map(|c| c.0);
                if let Some(c) = command {
                    select.run(s, field, c, tx);
                }
            }),
            on_submenu: Rc::new(|_: &mut S, _| {}),
            close_to: Rc::new(|_: &mut S, _| {}),
            on_dismiss: Rc::new(move |s: &mut S, _: &mut Tx| dismiss.finish(s, field)),
        }
        .build(layer)
    }

    /// The items, in order, each with whether it is sensitive.
    fn commands(
        &self,
        e: &TextEdit,
        clipboard: Clipboard,
        (undo, redo): (bool, bool),
    ) -> Vec<(FieldCommand, bool)> {
        let (a, b) = e.range();
        let selected = a != b;
        let secure = e.is_secure();
        let mut out = Vec::new();
        if self.undo {
            out.push((FieldCommand::Undo, self.editable && undo));
            out.push((FieldCommand::Redo, self.editable && redo));
        }
        out.extend([
            (FieldCommand::Cut, self.editable && selected && !secure),
            (FieldCommand::Copy, selected && !secure),
            (
                FieldCommand::Paste,
                self.editable && clipboard != Clipboard::Empty,
            ),
            (FieldCommand::Delete, self.editable && selected),
            (FieldCommand::SelectAll, !e.text.is_empty()),
        ]);
        out
    }

    /// Runs `command` on `field`, then closes the menu with the keyboard back on the field.
    fn run(&self, s: &mut S, field: F, command: FieldCommand, tx: &mut Tx) {
        let Some(e) = (self.edit_mut)(s, field) else {
            self.finish(s, field);
            return;
        };
        let changed = match command {
            FieldCommand::Undo => {
                e.undo();
                true
            }
            FieldCommand::Redo => {
                e.redo();
                true
            }
            FieldCommand::Cut => {
                tx.copy(e.selected().to_owned());
                e.insert("");
                true
            }
            FieldCommand::Copy => {
                tx.copy(e.selected().to_owned());
                false
            }
            FieldCommand::Delete => {
                e.insert("");
                true
            }
            FieldCommand::SelectAll => {
                e.set_selection(0, e.text.len());
                false
            }
            FieldCommand::Paste => {
                let (edit_mut, changed, pasted) =
                    (self.edit_mut, self.changed.clone(), self.pasted.clone());
                let asked = generation(e);
                tx.read_clipboard(move |s: &mut S, content, tx| {
                    let text = content.text();
                    let same = edit_mut(s, field).is_some_and(|e| generation(e) == asked);
                    if text.is_empty() || !same {
                        return;
                    }
                    if let Some(f) = &pasted {
                        f(s, field, text, tx);
                    } else if let Some(e) = edit_mut(s, field) {
                        e.paste(text);
                        if let Some(f) = &changed {
                            f(s, field, tx);
                        }
                    }
                });
                false
            }
        };
        if changed && let Some(f) = &self.changed {
            f(s, field, tx);
        }
        self.finish(s, field);
    }

    /// Closes the menu and gives the field the keyboard back.
    fn finish(&self, s: &mut S, field: F) {
        if (self.get_mut)(s).close().is_none() {
            return;
        }
        if let Some(e) = (self.edit_mut)(s, field) {
            e.focused = true;
        }
        if let Some(f) = &self.closed {
            f(s, field);
        }
    }
}

/// The menu's sections: Undo and Redo, then the edits, then Select All.
fn sections(commands: &[(FieldCommand, bool)], labels: &FieldLabels) -> Vec<Section> {
    let item = |&(c, on): &(FieldCommand, bool)| Item::new(c.id(), c.label(labels)).enabled(on);
    let part = |f: fn(FieldCommand) -> bool| {
        Section::new(commands.iter().filter(|(c, _)| f(*c)).map(item))
    };
    vec![
        part(|c| matches!(c, FieldCommand::Undo | FieldCommand::Redo)),
        part(|c| {
            matches!(
                c,
                FieldCommand::Cut | FieldCommand::Copy | FieldCommand::Paste | FieldCommand::Delete
            )
        }),
        part(|c| c == FieldCommand::SelectAll),
    ]
}

fn history(e: &TextEdit) -> (bool, bool) {
    let moved = |f: fn(&mut TextEdit)| {
        let mut c = e.clone();
        f(&mut c);
        c.text != e.text || c.range() != e.range()
    };
    (moved(TextEdit::undo), moved(TextEdit::redo))
}

fn generation(e: &TextEdit) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    e.text.hash(&mut h);
    e.range().hash(&mut h);
    h.finish()
}
