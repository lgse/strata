// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{glib, prelude::*};

use super::{ChordListener, ShortcutFooter, chord_panel::ChordPanel};
use crate::ui::tenxer_mode::Chord;

/// The search hit under the cursor, as its path below the searched folder.
/// The folder part gives way to an ellipsis before the name does.
#[derive(Clone)]
pub(super) struct CurrentHit {
    pub(super) root: gtk::Box,
    pub(super) folder: gtk::Label,
    pub(super) name: gtk::Label,
}

impl CurrentHit {
    const NAME_MIN_CHARS: usize = 32;

    pub(super) fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.set_visible(false);
        let folder = gtk::Label::new(None);
        folder.set_ellipsize(gtk::pango::EllipsizeMode::End);
        folder.set_max_width_chars(60);
        let name = gtk::Label::new(None);
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        for label in [&folder, &name] {
            label.add_css_class("shortcut-footer-chord-hint");
            root.append(label);
        }
        Self { root, folder, name }
    }

    pub(super) fn set(&self, path: Option<&std::path::Path>) {
        let Some(path) = path.filter(|path| path.file_name().is_some()) else {
            self.root.set_visible(false);
            return;
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let folder = path
            .parent()
            .map(|folder| folder.to_string_lossy())
            .unwrap_or_default();
        self.folder.set_text(&folder);
        self.folder.set_visible(!folder.is_empty());
        let name = if folder.is_empty() {
            name.into_owned()
        } else {
            format!("{}{name}", std::path::MAIN_SEPARATOR)
        };
        self.name.set_text(&name);
        let chars = name.chars().count();
        self.name
            .set_width_chars(chars.min(Self::NAME_MIN_CHARS) as i32);
        let full = path.to_string_lossy();
        crate::ui::accessibility::set_label(&self.root, &full);
        self.root.set_visible(true);
    }
}

/// The chord is armed only while its pill is visible.
#[derive(Clone)]
pub(super) struct ChordIndicator {
    pub(super) armed: Rc<Cell<Option<Chord>>>,
    pub(super) mark: glib::WeakRef<gtk::Label>,
    pub(super) panel: ChordPanel,
    pub(super) listeners: Rc<RefCell<Vec<ChordListener>>>,
}

impl ChordIndicator {
    pub(super) fn attach(mark: &gtk::Label) -> Self {
        Self {
            armed: Rc::new(Cell::new(None)),
            mark: mark.downgrade(),
            panel: ChordPanel::attach(mark),
            listeners: Rc::new(RefCell::new(Vec::new())),
        }
    }

    pub(super) fn clear(&self) {
        self.set_with(None, Vec::new());
    }

    /// Rows whose keys are unbound are left out.
    pub(super) fn set_with(&self, chord: Option<Chord>, mut rows: Vec<(String, String)>) {
        rows.retain(|(keys, _)| !keys.is_empty());
        let previous = self.armed.replace(chord);
        if let Some(mark) = self.mark.upgrade() {
            self.show_mark(&mark, &rows);
        }
        if previous != chord {
            for listener in self.listeners.borrow().iter() {
                listener(chord);
            }
        }
    }

    fn show_mark(&self, mark: &gtk::Label, rows: &[(String, String)]) {
        let chord = self.armed.get();
        mark.set_text(&chord.map(Chord::mark).unwrap_or_default());
        mark.set_visible(chord.is_some());
        match chord {
            Some(chord) if !rows.is_empty() => {
                self.panel.show(mark, chord, rows, self.armed.clone());
            }
            _ => self.panel.hide(),
        }
    }
}

/// Keep the experimental caveat available to assistive technology.
pub(super) fn apply_experimental_label(tag: &gtk::Label, enabled: bool) {
    tag.set_text(crate::ui::tenxer_mode::TAG_TEXT);
    tag.set_visible(enabled);
    let name = crate::i18n::tr(crate::ui::tenxer_mode::TAG_NAME);
    let phrase = crate::i18n::tr(crate::ui::shortcut_reference::EXPERIMENTAL_LABEL);
    let announced = if enabled {
        rust_i18n::t!(
            "%{mode} (experimental feature, under active development)",
            mode = name
        )
        .into_owned()
    } else {
        name
    };
    tag.update_property(&[
        gtk::accessible::Property::Label(&announced),
        gtk::accessible::Property::Description(if enabled { &phrase } else { "" }),
    ]);
}

pub(super) fn update_visual_mode(label: &gtk::Label, visual: Option<crate::app::VisualKind>) {
    let (text, name) = match visual {
        Some(crate::app::VisualKind::Select) => ("VISUAL", "Visual select"),
        Some(crate::app::VisualKind::Unset) => ("UNSET", "Visual unset"),
        None => ("", ""),
    };
    let (text, name) = if text.is_empty() {
        (String::new(), String::new())
    } else {
        (crate::i18n::tr(text), crate::i18n::tr(name))
    };
    if label.text() != text {
        label.set_text(&text);
        crate::ui::accessibility::set_label(label, &name);
    }
    label.set_visible(!text.is_empty());
}

impl ShortcutFooter {
    pub(in crate::ui) fn arm_chord(&self, chord: Chord) {
        let chooser = self.reference_scope.get().chooser.is_some();
        let rows = chord
            .options(chooser)
            .into_iter()
            .map(|(keys, action)| (keys.label(), crate::i18n::tr(action)))
            .collect();
        self.chords.set_with(Some(chord), rows);
    }

    pub(in crate::ui) fn arm_chord_with(&self, chord: Chord, rows: Vec<(String, String)>) {
        self.chords.set_with(Some(chord), rows);
    }

    pub(in crate::ui) fn armed_chord(&self) -> Option<Chord> {
        self.chords.armed.get()
    }

    pub(in crate::ui) fn cancel_chord(&self) {
        self.chords.clear();
    }

    pub(in crate::ui) fn connect_chord_changed(&self, listener: impl Fn(Option<Chord>) + 'static) {
        self.chords.listeners.borrow_mut().push(Box::new(listener));
    }

    #[cfg(test)]
    pub(in crate::ui) fn chord(&self) -> gtk::Label {
        self.chords.mark.upgrade().expect("chord mark")
    }

    #[cfg(test)]
    pub(in crate::ui) fn chord_options(&self) -> Option<Vec<(String, String)>> {
        self.chords.panel.shown_options()
    }

    #[cfg(test)]
    pub(in crate::ui) fn visual_text(&self) -> Option<String> {
        self.visual
            .is_visible()
            .then(|| self.visual.text().to_string())
    }

    #[cfg(test)]
    pub(in crate::ui) fn current_hit(&self) -> Option<(String, String)> {
        let current = &self.current;
        current.root.is_visible().then(|| {
            let folder = if current.folder.is_visible() {
                current.folder.text().to_string()
            } else {
                String::new()
            };
            (
                format!("{folder}{}", current.name.text()),
                format!("{folder}{}", current.name.text()),
            )
        })
    }
}
