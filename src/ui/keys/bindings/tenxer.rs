// SPDX-License-Identifier: MIT

use gtk::gdk::Key;

use crate::ui::keys::{Binding, Command, Digits};

catalog! {
    tenxer_only {
        CLOSE_WINDOW: Command "close_window" = Binding::shift(Key::q);

        OPEN: Command "open" = Binding::plain(Key::o);
        HALF_PAGE_UP: Command "half_page_up" = Binding::ctrl(Key::u);
        HALF_PAGE_DOWN: Command "half_page_down" = Binding::ctrl(Key::d);

        YANK: Command "yank" = Binding::plain(Key::y);
        YANK_CUT: Command "yank_cut" = Binding::plain(Key::x);
        UNYANK: Command "unyank" => [Binding::shift(Key::y), Binding::shift(Key::x)];
        PASTE_KEEP_BOTH: Command "paste_keep_both" = Binding::plain(Key::p);
        PASTE_REPLACE: Command "paste_replace" = Binding::shift(Key::p);
        CREATE: Command "create" = Binding::plain(Key::a);
        OPEN_WITH: Command "open_with" = Binding::shift(Key::o);
        MOVE_TO: Command "move_to" = Binding::shift(Key::m);
        COPY_TO: Command "copy_to" = Binding::shift(Key::c);
        RESTORE: Command "restore" = Binding::shift(Key::r);

        VISUAL_SELECT: Command "visual_select" = Binding::plain(Key::v);
        VISUAL_UNSET: Command "visual_unset" = Binding::shift(Key::v);
        INVERT_SELECTION: Command "invert_selection" = Binding::ctrl(Key::r);

        FIND: Command "find" = Binding::plain(Key::slash);
        FIND_BACKWARD: Command "find_backward" = Binding::plain(Key::question);
        REPEAT_FIND: Command "repeat_find" = Binding::plain(Key::n);
        REPEAT_FIND_REVERSED: Command "repeat_find_reversed" = Binding::shift(Key::n);
        SEARCH_SUBFOLDERS: Command "search_subfolders" = Binding::plain(Key::s);
        JUMP_VISITED: Command "jump_visited" = Binding::plain(Key::z);
        JUMP_RECENT: Command "jump_recent" = Binding::shift(Key::z);

        GO: Command "go" trigger(Go) = Binding::plain(Key::g);
        GO_FIRST: Command "go.first" step(Go) = Binding::plain(Key::g);
        GO_FOLLOW: Command "go.follow" step(Go) = Binding::plain(Key::f);
        GO_PROMPT: Command "go.prompt" step(Go) = Binding::plain(Key::space);
        GO_HOME: Command "go.home" step(Go) = Binding::plain(Key::h);
        GO_CONFIG: Command "go.config" step(Go) = Binding::plain(Key::c);
        GO_DOWNLOADS: Command "go.downloads" step(Go) = Binding::plain(Key::d);
        GO_DOCUMENTS: Command "go.documents" step(Go) = Binding::plain(Key::k);
        GO_MUSIC: Command "go.music" step(Go) = Binding::plain(Key::m);
        GO_PICTURES: Command "go.pictures" step(Go) = Binding::plain(Key::p);
        GO_VIDEOS: Command "go.videos" step(Go) = Binding::plain(Key::v);
        GO_TRASH: Command "go.trash" step(Go) = Binding::plain(Key::t);
        GO_NETWORK: Command "go.network" step(Go) = Binding::plain(Key::n);
        GO_RECENT: Command "go.recent" step(Go) = Binding::plain(Key::r);
        GO_PIN: Command "go.pin" step(Go) = Binding::plain(Key::plus);
        GO_UNPIN: Command "go.unpin" step(Go) = Binding::plain(Key::minus);
        PIN_NUMBERS: Digits = Digits::after(GO.shown(), 9, false);

        SORT_BY: Command "sort" trigger(Sort) = Binding::plain(Key::comma);
        SORT_BY_NAME: Command "sort.name" step(Sort) = Binding::plain(Key::a);
        SORT_BY_MODIFIED: Command "sort.modified" step(Sort) = Binding::plain(Key::m);
        SORT_BY_SIZE: Command "sort.size" step(Sort) = Binding::plain(Key::s);
        SORT_BY_TYPE: Command "sort.type" step(Sort) = Binding::plain(Key::e);

        COPY_AS: Command "copy_as" trigger(CopyAs) = Binding::plain(Key::c);
        COPY_AS_PATH: Command "copy_as.path" step(CopyAs) = Binding::plain(Key::c);
        COPY_AS_NAME: Command "copy_as.name" step(CopyAs) = Binding::plain(Key::n);

        ACTIONS: Command "actions" trigger(Actions) = Binding::plain(Key::semicolon);
        ACTIONS_TERMINAL: Command "actions.terminal" step(Actions) = Binding::plain(Key::t);
        ACTIONS_COMPRESS: Command "actions.compress" step(Actions) = Binding::plain(Key::c);
        ACTIONS_EXTRACT: Command "actions.extract" step(Actions) = Binding::plain(Key::e);
        ACTIONS_EXTRACT_TO: Command "actions.extract_to" step(Actions) = Binding::shift(Key::e);
        ACTIONS_NUMBERS: Digits = Digits::after(ACTIONS.shown(), 9, true);

        TAB_CHORD: Command "tabs" trigger(Tabs) = Binding::plain(Key::t);
        TAB_CHORD_NEW: Command "tabs.new" step(Tabs) = Binding::plain(Key::n);
        TAB_CHORD_CLOSE: Command "tabs.close" step(Tabs) = Binding::plain(Key::x);
        TAB_CHORD_PREVIOUS: Command "tabs.previous" step(Tabs) = Binding::plain(Key::t);
        TAB_CHORD_NUMBERS: Digits = Digits::after(TAB_CHORD.shown(), 9, true);
    }
}
