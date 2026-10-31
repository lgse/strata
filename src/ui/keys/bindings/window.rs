// SPDX-License-Identifier: MIT

use gtk::gdk::{Key, ModifierType as Modifiers};

use crate::ui::keys::{Binding, Command, Digits};

catalog! {
    listed {
        COPY: Command "copy" = Binding::ctrl(Key::c);
        CUT: Command "cut" = Binding::ctrl(Key::x);
        PASTE: Command "paste" = Binding::ctrl(Key::v);
        SELECT_ALL: Command "select_all" = Binding::ctrl(Key::a);
        UNDO: Command "undo" = Binding::ctrl(Key::z);
        REDO: Command "redo" => [Binding::ctrl_shift(Key::z), Binding::ctrl(Key::y)];

        NEW_FOLDER: Command "new_folder" = Binding::ctrl_shift(Key::n);
        NEW_FOLDER_WITH_SELECTION: Command "new_folder_with_selection" =
            Binding::ctrl_alt(Key::n);
        RENAME: Command "rename" => default [Binding::plain(Key::F2), Binding::ctrl(Key::r)],
            tenxer [Binding::plain(Key::r), Binding::plain(Key::F2)];
        PROPERTIES: Command "properties" = Binding::alt(Key::Return);
        CONTEXT_MENU: Command "context_menu" =>
            [Binding::plain(Key::Menu), Binding::shift(Key::F10)];
        MOVE_TO_TRASH: Command "move_to_trash" => default [Binding::plain(Key::Delete)],
            tenxer [Binding::plain(Key::d), Binding::plain(Key::Delete)];
        DELETE_PERMANENTLY: Command "delete_permanently" =>
            default [Binding::shift(Key::Delete)],
            tenxer [Binding::shift(Key::d), Binding::shift(Key::Delete)];

        SEARCH: Command "search" = Binding::ctrl(Key::k);
        LOCATION: Command "location" = Binding::ctrl(Key::l);
        REFRESH: Command "refresh" = Binding::plain(Key::F5);
        /// Menus show Ctrl+H in both maps.
        HIDDEN_FILES: Command "toggle_hidden_files" =>
            default [Binding::ctrl(Key::h), Binding::ctrl(Key::period)],
            tenxer [
                Binding::ctrl(Key::h),
                Binding::ctrl(Key::period),
                Binding::plain(Key::period),
            ];
        FILTER: Command "filter" => default [Binding::ctrl(Key::f)],
            tenxer [Binding::plain(Key::f)];

        VIEW_COLUMNS: Command "view_columns" = Binding::ctrl(Key::_1);
        VIEW_ICONS: Command "view_icons" = Binding::ctrl(Key::_2);
        VIEW_LIST: Command "view_list" = Binding::ctrl(Key::_3);
        /// US keyboards type **+** with Shift+=, so Ctrl+= also enlarges.
        TEXT_LARGER: Command "text_larger" =>
            [Binding::ctrl(Key::plus), Binding::ctrl(Key::equal)];
        TEXT_SMALLER: Command "text_smaller" = Binding::ctrl(Key::minus);
        TEXT_RESET: Command "text_reset" = Binding::ctrl(Key::_0);
        THUMBNAILS_LARGER: Command "thumbnails_larger" =>
            default [Binding::ctrl_alt(Key::plus), Binding::ctrl_alt(Key::equal)],
            tenxer [
                Binding::ctrl_alt(Key::plus),
                Binding::ctrl_alt(Key::equal),
                Binding::plain(Key::plus),
                Binding::plain(Key::equal),
            ];
        THUMBNAILS_SMALLER: Command "thumbnails_smaller" =>
            default [Binding::ctrl_alt(Key::minus)],
            tenxer [Binding::ctrl_alt(Key::minus), Binding::plain(Key::minus)];
        THUMBNAILS_RESET: Command "thumbnails_reset" = Binding::ctrl_alt(Key::_0);
        SIDEBAR: Command "toggle_sidebar" => default [Binding::ctrl(Key::b)],
            tenxer [Binding::ctrl(Key::n)];
        SIDEBAR_FOCUS: Command "focus_sidebar" = Binding::ctrl_shift(Key::b);

        SETTINGS: Command "settings" = Binding::ctrl(Key::comma);
        REFERENCE: Command "shortcut_reference" => default [Binding::plain(Key::F1)],
            tenxer [Binding::plain(Key::F1), Binding::plain(Key::asciitilde)];
        TENXER_MODE: Command "tenxer_mode" = Binding::ctrl_shift(Key::m);

        TAB_NEW: Command "tab_new" = Binding::ctrl(Key::t);
        TAB_CLOSE: Command "tab_close" = Binding::ctrl(Key::w);
        TAB_REOPEN: Command "tab_reopen" = Binding::ctrl_shift(Key::t);
        TAB_NEXT: Command "tab_next" =>
            [Binding::ctrl(Key::Tab), Binding::ctrl(Key::Page_Down)];
        TAB_PREVIOUS: Command "tab_previous" =>
            [Binding::ctrl_shift(Key::Tab), Binding::ctrl(Key::Page_Up)];
        TAB_MOVE_LEFT: Command "tab_move_left" = Binding::ctrl_shift(Key::Page_Up);
        TAB_MOVE_RIGHT: Command "tab_move_right" = Binding::ctrl_shift(Key::Page_Down);
        TAB_NUMBERS: Digits = Digits::new(
            Modifiers::CONTROL_MASK.union(Modifiers::SHIFT_MASK),
            9,
            true,
        );
    }
    default_only {
        // The 10xer map gives these keys to other commands or leaves them out.
        DUPLICATE: Command "duplicate" = Binding::ctrl(Key::d);
        COPY_PATH: Command "copy_path" = Binding::plain(Key::y);
        PIN: Command "pin" = Binding::plain(Key::p);
        JUMP_TO_FOLDER: Command "jump_to_folder" = Binding::ctrl_shift(Key::k);
        TERMINAL: Command "terminal" = Binding::ctrl_alt(Key::t), live in both;
        ARROW_SCOPE: Command "arrow_scope" = Binding::ctrl(Key::backslash);
        // Ship no key; slot N runs the Nth action Settings → Actions lists.
        CUSTOM_ACTION_1: Command "custom_action.1" => default [], tenxer [];
        CUSTOM_ACTION_2: Command "custom_action.2" => default [], tenxer [];
        CUSTOM_ACTION_3: Command "custom_action.3" => default [], tenxer [];
        CUSTOM_ACTION_4: Command "custom_action.4" => default [], tenxer [];
        CUSTOM_ACTION_5: Command "custom_action.5" => default [], tenxer [];
        CUSTOM_ACTION_6: Command "custom_action.6" => default [], tenxer [];
        CUSTOM_ACTION_7: Command "custom_action.7" => default [], tenxer [];
        CUSTOM_ACTION_8: Command "custom_action.8" => default [], tenxer [];
        CUSTOM_ACTION_9: Command "custom_action.9" => default [], tenxer [];
        CUSTOM_ACTION_10: Command "custom_action.10" => default [], tenxer [];
    }
    unlisted {
        /// While type-to-search is on, **/** opens the filter without typing.
        FILTER_SLASH: Command "type_to_search_filter" = Binding::plain(Key::slash),
            live in default;
    }
}
