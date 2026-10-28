// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType as Modifiers};

use crate::ui::keys::{Binding, Command, Matcher, SHORTCUT_MODIFIERS};

catalog! {
    listed {
        // h j k l are read as arrows outside text fields and popovers, and
        // in the default map only while type-to-search is off.
        UP: Command "up" structural => [Binding::plain(Key::Up), Binding::plain(Key::k)];
        DOWN: Command "down" structural => [Binding::plain(Key::Down), Binding::plain(Key::j)];
        LEFT: Command "left" structural => [Binding::plain(Key::Left), Binding::plain(Key::h)];
        RIGHT: Command "right" structural =>
            [Binding::plain(Key::Right), Binding::plain(Key::l)];
        FIRST_ITEM: Command "first_item" = Binding::plain(Key::Home);
        LAST_ITEM: Command "last_item" => default [Binding::plain(Key::End)],
            tenxer [Binding::shift(Key::g), Binding::plain(Key::End)];
        FIRST_ITEM_UP: Command "jump_to_first_item" = Binding::ctrl(Key::Up);
        LAST_ITEM_DOWN: Command "jump_to_last_item" = Binding::ctrl(Key::Down);
        // Where no tab strip takes them first, as in file choosers, Ctrl with
        // the page keys pages too.
        PAGE_UP: Command "page_up" => default [Binding::plain(Key::Page_Up)],
            tenxer [
                Binding::plain(Key::Page_Up),
                Binding::ctrl(Key::b),
                Binding::ctrl(Key::Page_Up),
            ];
        PAGE_DOWN: Command "page_down" => default [Binding::plain(Key::Page_Down)],
            tenxer [
                Binding::plain(Key::Page_Down),
                Binding::ctrl(Key::f),
                Binding::ctrl(Key::Page_Down),
            ];

        ENTER: Command "activate" structural = Binding::plain(Key::Return);
        ESCAPE: Command "escape" structural = Binding::plain(Key::Escape);
        BACKSPACE: Binding = Binding::plain(Key::BackSpace);
        FOCUS_NEXT: Command "focus_next" structural = Binding::plain(Key::Tab);
        FOCUS_PREVIOUS: Command "focus_previous" structural = Binding::shift(Key::Tab);

        HISTORY_BACK: Command "history_back" => default [Binding::alt(Key::Left)],
            tenxer [Binding::alt(Key::Left), Binding::shift(Key::h)];
        HISTORY_FORWARD: Command "history_forward" => default [Binding::alt(Key::Right)],
            tenxer [Binding::alt(Key::Right), Binding::shift(Key::l)];
        PARENT_FOLDER: Command "parent_folder" = Binding::alt(Key::Up);
        HOME_FOLDER: Command "home_folder" = Binding::alt(Key::Home);
        OPEN_IN_NEW_TAB: Command "open_in_new_tab" = Binding::ctrl(Key::Return);
        OPEN_IN_NEW_WINDOW: Command "open_in_new_window" = Binding::shift(Key::Return);
        /// Previews the focused file or opens a folder; 10xer peeks with **i**.
        PREVIEW: Command "preview" => default [Binding::plain(Key::space)],
            tenxer [Binding::plain(Key::i)];
    }
    default_only {
        EXTEND_UP: Binding = Binding::shift(Key::Up);
        EXTEND_DOWN: Binding = Binding::shift(Key::Down);
        EXTEND_PAGE_UP: Binding = Binding::shift(Key::Page_Up);
        EXTEND_PAGE_DOWN: Binding = Binding::shift(Key::Page_Down);
        /// GTK's list views handle it; the reference names it.
        SELECTION_TOGGLE: Binding = Binding::ctrl(Key::space);
    }
    tenxer_only {
        /// Space wherever it activates, selects, plays or types; [`PREVIEW`]
        /// is its quick-preview job, and all the default reference shows.
        SPACE: Binding = Binding::plain(Key::space);
        /// Tab where it completes a path rather than moving focus. Location bar
        /// completion reads it in either map.
        COMPLETE: Command "complete" = Binding::plain(Key::Tab), live in both;
    }
    unlisted {
        // Range selection that GTK performs once the dispatcher passes the key on.
        EXTEND_LEFT: Binding = Binding::shift(Key::Left);
        EXTEND_RIGHT: Binding = Binding::shift(Key::Right);
        EXTEND_TO_FIRST: Binding = Binding::shift(Key::Home);
        EXTEND_TO_LAST: Binding = Binding::shift(Key::End);

        // Home, End and the page keys where they move a caret, seek, scroll or
        // page a list other than the file listing, which the item commands own.
        HOME_KEY: Binding = Binding::plain(Key::Home);
        END_KEY: Binding = Binding::plain(Key::End);
        PAGE_UP_KEY: Binding = Binding::plain(Key::Page_Up);
        PAGE_DOWN_KEY: Binding = Binding::plain(Key::Page_Down);
    }
}

/// Escape with any modifiers, as dialogs and inline edits accept it.
pub(crate) const ESCAPE_ANY: Matcher = ESCAPE.ignoring(SHORTCUT_MODIFIERS);
/// Escape with or without Shift, as the listing, prompts and popovers accept it.
pub(crate) const ESCAPE_ANY_SHIFT: Matcher = ESCAPE.ignoring(Modifiers::SHIFT_MASK);
/// Enter with any modifiers, as confirmation dialogs accept it.
pub(crate) const ENTER_ANY: Matcher = ENTER.ignoring(SHORTCUT_MODIFIERS);
/// Enter with or without Shift, as the listing, prompts and popovers accept it.
pub(crate) const ENTER_ANY_SHIFT: Matcher = ENTER.ignoring(Modifiers::SHIFT_MASK);
