// SPDX-License-Identifier: MIT

use std::mem::discriminant;

use gtk::gdk::{Key, ModifierType as Modifiers};

use super::{
    super::{
        Binding, Command, Keymap, Map,
        bindings::{
            ACTIONS_NUMBERS, ACTIONS_TERMINAL, CHOOSE_FOLDER, COPY, COPY_AS_NAME, COPY_PATH,
            DELETE_PERMANENTLY, DOWN, DUPLICATE, ENTER, ESCAPE, FILTER, FILTER_SLASH, FIND,
            FIND_IN_PREVIEW, GO, GO_DOWNLOADS, GO_FIRST, GO_HOME, GO_PROMPT, HISTORY_BACK,
            LAST_ITEM, LEFT, MEDIA_SEEK_BACK_PAGE, MEDIA_SEEK_END, MEDIA_SEEK_START,
            MEDIA_VOLUME_STEP_DOWN, MEDIA_VOLUME_STEP_UP, MOVE_TO_TRASH, NEW_FOLDER,
            OPEN_CONTAINING_FOLDER, OPEN_IN_NEW_WINDOW, OPEN_WITH, PAGE_DOWN, PAGE_UP,
            PDF_ZOOM_RESET, PIN, PIN_NUMBERS, PREVIEW, PREVIOUS_MATCH, REFERENCE, REFERENCE_SEARCH,
            REFRESH, RENAME, RIGHT, SORT_BY_NAME, TAB_NEW, TAB_NEXT, TAB_NUMBERS, TAB_PREVIOUS,
            TENXER_DELETE_CONFIRM, TENXER_MEDIA_MUTE, TENXER_MEDIA_PREVIOUS, TENXER_MODE, TERMINAL,
            TEXT_LARGER, TEXT_RESET, THUMBNAILS_LARGER, THUMBNAILS_SMALLER, UP, YANK,
        },
        command::Group,
    },
    FieldLimit, Finding, Fixed, Kind, Policy, Severity, audit, check,
};

const OFF: Policy = Policy {
    type_to_search: false,
};
const ON: Policy = Policy {
    type_to_search: true,
};
const SUPER: Modifiers = Modifiers::SUPER_MASK;

struct Row {
    case: &'static str,
    map: Map,
    command: Command,
    candidate: Binding,
    /// Entries replaced in the table before the check.
    table: &'static [(Command, &'static [Binding])],
    policy: Policy,
    kind: Kind,
    /// Whether the check reports `kind`; a row that expects none means no
    /// finding of that kind, whatever its payload.
    found: bool,
}

const ROWS: &[Row] = &[
    Row {
        case: "a key another command holds",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::ctrl(Key::c),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(COPY),
        found: true,
    },
    Row {
        case: "a free key",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::ctrl(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(COPY),
        found: false,
    },
    Row {
        case: "a plain digit accepts the Shift a Shifted digit holds",
        map: Map::Default,
        command: PIN,
        candidate: Binding::shift(Key::_1),
        table: &[(
            RENAME,
            &[Binding::plain(Key::_1).ignoring(Modifiers::SHIFT_MASK)],
        )],
        policy: OFF,
        kind: Kind::Duplicate(RENAME),
        found: true,
    },
    Row {
        case: "a step on another step of its group",
        map: Map::Tenxer,
        command: GO_HOME,
        candidate: Binding::plain(Key::d),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(GO_DOWNLOADS),
        found: true,
    },
    Row {
        case: "a step on a top-level key",
        map: Map::Tenxer,
        command: GO_HOME,
        candidate: Binding::plain(Key::y),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(COPY),
        found: false,
    },
    Row {
        case: "a step on its own trigger's key, as g g",
        map: Map::Tenxer,
        command: GO_HOME,
        candidate: Binding::plain(Key::g),
        table: &[(GO_FIRST, &[])],
        policy: OFF,
        kind: Kind::Duplicate(GO),
        found: false,
    },
    Row {
        case: "a step on a key the armed chord reads as Escape",
        map: Map::Tenxer,
        command: GO_HOME,
        candidate: Binding::plain(Key::q),
        table: &[(
            ESCAPE,
            &[Binding::plain(Key::Escape), Binding::plain(Key::q)],
        )],
        policy: OFF,
        kind: Kind::Duplicate(ESCAPE),
        found: true,
    },
    Row {
        case: "an Escape alternate on a Shift variant a step tolerates",
        map: Map::Tenxer,
        command: ESCAPE,
        candidate: Binding::shift(Key::space),
        table: &[],
        policy: OFF,
        kind: Kind::ContextOverlap(GO_PROMPT),
        found: true,
    },
    Row {
        case: "a step on a key the reference takes before an armed chord",
        map: Map::Tenxer,
        command: GO_HOME,
        candidate: Binding::plain(Key::asciitilde),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(REFERENCE),
        found: true,
    },
    Row {
        case: "a media key the reference takes before a media preview",
        map: Map::Tenxer,
        command: TENXER_MEDIA_MUTE,
        candidate: Binding::plain(Key::F1),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(REFERENCE),
        found: true,
    },
    Row {
        case: "a reference key Escape takes, closing the reference first",
        map: Map::Default,
        command: REFERENCE_SEARCH,
        candidate: Binding::plain(Key::Escape),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(ESCAPE),
        found: true,
    },
    Row {
        case: "a preview command on a text size key, which runs first",
        map: Map::Default,
        command: FIND_IN_PREVIEW,
        candidate: Binding::ctrl(Key::_0),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(TEXT_RESET),
        found: true,
    },
    Row {
        case: "a preview command on a tab key, which the tab strip takes first",
        map: Map::Default,
        command: FIND_IN_PREVIEW,
        candidate: Binding::ctrl(Key::t),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(TAB_NEW),
        found: true,
    },
    Row {
        case: "a tab key in global search, which blocks the tab strip",
        map: Map::Default,
        command: OPEN_CONTAINING_FOLDER,
        candidate: Binding::ctrl(Key::t),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(TAB_NEW),
        found: false,
    },
    Row {
        case: "a step on a tab key, which the tab strip takes over an armed chord",
        map: Map::Tenxer,
        command: GO_HOME,
        candidate: Binding::ctrl(Key::t),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(TAB_NEW),
        found: true,
    },
    Row {
        case: "the document zoom reset on a text size key, which it runs before",
        map: Map::Default,
        command: PDF_ZOOM_RESET,
        candidate: Binding::ctrl(Key::plus),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(TEXT_LARGER),
        found: false,
    },
    Row {
        case: "text size where a focused document resets its zoom first",
        map: Map::Default,
        command: PDF_ZOOM_RESET,
        candidate: Binding::ctrl(Key::plus),
        table: &[],
        policy: OFF,
        kind: Kind::ContextOverlap(TEXT_LARGER),
        found: true,
    },
    Row {
        case: "a Go step where a focused document resets its zoom first",
        map: Map::Tenxer,
        command: GO_HOME,
        candidate: Binding::ctrl(Key::e),
        table: &[(PDF_ZOOM_RESET, &[Binding::ctrl(Key::e)])],
        policy: OFF,
        kind: Kind::ContextOverlap(PDF_ZOOM_RESET),
        found: true,
    },
    Row {
        case: "a key the chooser renames with before it accepts a folder",
        map: Map::Default,
        command: CHOOSE_FOLDER,
        candidate: Binding::ctrl(Key::r),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(RENAME),
        found: true,
    },
    Row {
        case: "an Escape alternate, which the chooser reads before it accepts a folder",
        map: Map::Default,
        command: CHOOSE_FOLDER,
        candidate: Binding::ctrl(Key::bracketleft),
        table: &[(
            ESCAPE,
            &[Binding::plain(Key::Escape), Binding::ctrl(Key::bracketleft)],
        )],
        policy: OFF,
        kind: Kind::Duplicate(ESCAPE),
        found: true,
    },
    Row {
        case: "a key the chooser refreshes with after it accepts a folder",
        map: Map::Default,
        command: CHOOSE_FOLDER,
        candidate: Binding::plain(Key::F5),
        table: &[],
        policy: OFF,
        kind: Kind::ContextOverlap(REFRESH),
        found: true,
    },
    Row {
        case: "a key the 10xer stage claims before the chooser's own",
        map: Map::Tenxer,
        command: CHOOSE_FOLDER,
        candidate: Binding::plain(Key::F2),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(RENAME),
        found: true,
    },
    Row {
        case: "a key the 10xer stage leaves to the chooser",
        map: Map::Tenxer,
        command: CHOOSE_FOLDER,
        candidate: Binding::plain(Key::F5),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(REFRESH),
        found: false,
    },
    Row {
        case: "a clipboard key the 10xer stage refuses in the chooser",
        map: Map::Tenxer,
        command: CHOOSE_FOLDER,
        candidate: Binding::ctrl(Key::c),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(COPY),
        found: true,
    },
    Row {
        case: "a window key the chooser never reads",
        map: Map::Default,
        command: CHOOSE_FOLDER,
        candidate: Binding::ctrl(Key::t),
        table: &[],
        policy: OFF,
        kind: Kind::ContextOverlap(TAB_NEW),
        found: false,
    },
    Row {
        case: "a top-level key on a 10xer trigger",
        map: Map::Tenxer,
        command: RENAME,
        candidate: Binding::plain(Key::g),
        table: &[],
        policy: OFF,
        kind: Kind::ShadowsChordGroup(Group::Go),
        found: true,
    },
    Row {
        case: "a top-level key the trigger moved away from",
        map: Map::Tenxer,
        command: RENAME,
        candidate: Binding::plain(Key::g),
        table: &[(GO, &[Binding::plain(Key::q)])],
        policy: OFF,
        kind: Kind::ShadowsChordGroup(Group::Go),
        found: false,
    },
    Row {
        case: "a Shifted Sort step",
        map: Map::Tenxer,
        command: SORT_BY_NAME,
        candidate: Binding::shift(Key::b),
        table: &[],
        policy: OFF,
        kind: Kind::ShiftedSortStep,
        found: true,
    },
    Row {
        case: "an unshifted Sort step",
        map: Map::Tenxer,
        command: SORT_BY_NAME,
        candidate: Binding::plain(Key::b),
        table: &[],
        policy: OFF,
        kind: Kind::ShiftedSortStep,
        found: false,
    },
    Row {
        case: "a direct sort key with Shift, which nothing reverses",
        map: Map::Default,
        command: SORT_BY_NAME,
        candidate: Binding::ctrl_shift(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::ShiftedSortStep,
        found: false,
    },
    Row {
        case: "a Sort step on a digit",
        map: Map::Tenxer,
        command: SORT_BY_NAME,
        candidate: Binding::plain(Key::_1),
        table: &[],
        policy: OFF,
        kind: Kind::SortStepCannotReverse,
        found: true,
    },
    Row {
        case: "a Sort step on a symbol Shift turns into another",
        map: Map::Tenxer,
        command: SORT_BY_NAME,
        candidate: Binding::plain(Key::slash),
        table: &[],
        policy: OFF,
        kind: Kind::SortStepCannotReverse,
        found: true,
    },
    Row {
        case: "a Sort step on a function key",
        map: Map::Tenxer,
        command: SORT_BY_NAME,
        candidate: Binding::plain(Key::F3),
        table: &[],
        policy: OFF,
        kind: Kind::SortStepCannotReverse,
        found: false,
    },
    Row {
        case: "a plain letter while type-to-search is on",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::plain(Key::e),
        table: &[],
        policy: ON,
        kind: Kind::TypeToSearchShadowed,
        found: true,
    },
    Row {
        case: "an added digit on a preview command while type-to-search is on",
        map: Map::Default,
        command: MEDIA_SEEK_START,
        candidate: Binding::plain(Key::_9),
        table: &[],
        policy: ON,
        kind: Kind::TypeToSearchShadowed,
        found: true,
    },
    Row {
        case: "a plain letter while type-to-search is off",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::plain(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::TypeToSearchShadowed,
        found: false,
    },
    Row {
        case: "a plain letter in 10xer",
        map: Map::Tenxer,
        command: RENAME,
        candidate: Binding::plain(Key::e),
        table: &[],
        policy: ON,
        kind: Kind::TypeToSearchShadowed,
        found: false,
    },
    Row {
        case: "Space, which type-to-search leaves to preview",
        map: Map::Default,
        command: PREVIEW,
        candidate: Binding::plain(Key::space),
        table: &[],
        policy: ON,
        kind: Kind::TypeToSearchShadowed,
        found: false,
    },
    Row {
        case: "the slash that opens the filter while type-to-search is on",
        map: Map::Default,
        command: FILTER_SLASH,
        candidate: Binding::plain(Key::slash),
        table: &[],
        policy: ON,
        kind: Kind::TypeToSearchShadowed,
        found: false,
    },
    Row {
        case: "a plain letter for Enter",
        map: Map::Default,
        command: ENTER,
        candidate: Binding::plain(Key::o),
        table: &[],
        policy: OFF,
        kind: Kind::TextFieldLimit(FieldLimit::Typing),
        found: true,
    },
    Row {
        case: "a key text fields bind for an arrow",
        map: Map::Default,
        command: DOWN,
        candidate: Binding::ctrl(Key::BackSpace),
        table: &[],
        policy: OFF,
        kind: Kind::TextFieldLimit(FieldLimit::FieldKey),
        found: true,
    },
    Row {
        case: "a Ctrl letter for Enter",
        map: Map::Default,
        command: ENTER,
        candidate: Binding::ctrl(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::TextFieldLimit(FieldLimit::Typing),
        found: false,
    },
    Row {
        case: "an added plain letter, which the reference's search types",
        map: Map::Tenxer,
        command: REFERENCE,
        candidate: Binding::plain(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::TextFieldLimit(FieldLimit::Typing),
        found: true,
    },
    Row {
        case: "an added plain letter on a 10xer window command, which a text preview passes",
        map: Map::Tenxer,
        command: RENAME,
        candidate: Binding::plain(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::TextFieldLimit(FieldLimit::Typing),
        found: false,
    },
    Row {
        case: "Ctrl with the page keys pages where no tab strip takes them",
        map: Map::Tenxer,
        command: PAGE_UP,
        candidate: Binding::ctrl(Key::Page_Up),
        table: &[],
        policy: OFF,
        kind: Kind::ContextOverlap(TAB_PREVIOUS),
        found: true,
    },
    Row {
        case: "an added plain letter on the 10xer terminal, which text fields do not run",
        map: Map::Tenxer,
        command: TERMINAL,
        candidate: Binding::plain(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::TextFieldLimit(FieldLimit::Typing),
        found: false,
    },
    Row {
        case: "an added plain letter on the default terminal, which text fields keep",
        map: Map::Default,
        command: TERMINAL,
        candidate: Binding::plain(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::TextFieldLimit(FieldLimit::Typing),
        found: true,
    },
    Row {
        case: "a modified Escape, which Escape's modifier-tolerant matchers also read",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::alt(Key::Escape),
        table: &[],
        policy: OFF,
        kind: Kind::ContextOverlap(ESCAPE),
        found: true,
    },
    Row {
        case: "Shift held with an Enter alternate reads as another command",
        map: Map::Default,
        command: ENTER,
        candidate: Binding::ctrl(Key::m),
        table: &[],
        policy: OFF,
        kind: Kind::ContextOverlap(TENXER_MODE),
        found: true,
    },
    Row {
        case: "Alt held where trash tolerates it",
        map: Map::Default,
        command: DELETE_PERMANENTLY,
        candidate: Binding::alt(Key::x),
        table: &[(MOVE_TO_TRASH, &[Binding::plain(Key::x)])],
        policy: OFF,
        kind: Kind::ContextOverlap(MOVE_TO_TRASH),
        found: true,
    },
    Row {
        case: "an arrow alternate on a key the preview reads",
        map: Map::Tenxer,
        command: DOWN,
        candidate: Binding::plain(Key::less),
        table: &[],
        policy: OFF,
        kind: Kind::ContextOverlap(TENXER_MEDIA_PREVIOUS),
        found: true,
    },
    Row {
        case: "a key the listing reads while a media preview is open",
        map: Map::Tenxer,
        command: RENAME,
        candidate: Binding::plain(Key::less),
        table: &[],
        policy: OFF,
        kind: Kind::ContextOverlap(TENXER_MEDIA_PREVIOUS),
        found: true,
    },
    Row {
        case: "a window key a focused preview reads first",
        map: Map::Default,
        command: FIND_IN_PREVIEW,
        candidate: Binding::ctrl(Key::r),
        table: &[],
        policy: OFF,
        kind: Kind::ContextOverlap(RENAME),
        found: true,
    },
    Row {
        case: "a media command on Up, which a media preview reads as volume first",
        map: Map::Tenxer,
        command: TENXER_MEDIA_MUTE,
        candidate: Binding::plain(Key::Up),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(UP),
        found: true,
    },
    Row {
        case: "mute on Left, which a media preview reads before seeking",
        map: Map::Tenxer,
        command: TENXER_MEDIA_MUTE,
        candidate: Binding::plain(Key::Left),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(LEFT),
        found: false,
    },
    Row {
        case: "Ctrl+Space toggles selection in list views",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::ctrl(Key::space),
        table: &[],
        policy: OFF,
        kind: Kind::ReservedByGtk,
        found: true,
    },
    Row {
        case: "Shift+Up extends the selection",
        map: Map::Tenxer,
        command: RENAME,
        candidate: Binding::shift(Key::Up),
        table: &[],
        policy: OFF,
        kind: Kind::ReservedByGtk,
        found: true,
    },
    Row {
        case: "Shift+Left seeks in the media timeline",
        map: Map::Tenxer,
        command: MEDIA_SEEK_BACK_PAGE,
        candidate: Binding::shift(Key::Left),
        table: &[],
        policy: OFF,
        kind: Kind::ReservedByGtk,
        found: false,
    },
    Row {
        case: "an input method switch key for Enter",
        map: Map::Default,
        command: ENTER,
        candidate: Binding::new(Key::space, SUPER),
        table: &[],
        policy: OFF,
        kind: Kind::InputMethodKey,
        found: true,
    },
    Row {
        case: "an added punctuation key, which yields to a focused field",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::plain(Key::semicolon),
        table: &[],
        policy: OFF,
        kind: Kind::InputMethodKey,
        found: false,
    },
    Row {
        case: "an input method switch key on a command text fields never run",
        map: Map::Default,
        command: COPY_PATH,
        candidate: Binding::new(Key::space, SUPER),
        table: &[],
        policy: OFF,
        kind: Kind::InputMethodKey,
        found: false,
    },
    Row {
        case: "Alt with a letter",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::alt(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::ChooserMnemonic,
        found: true,
    },
    Row {
        case: "Alt with an arrow",
        map: Map::Default,
        command: HISTORY_BACK,
        candidate: Binding::alt(Key::Left),
        table: &[],
        policy: OFF,
        kind: Kind::ChooserMnemonic,
        found: false,
    },
    Row {
        case: "Alt with a letter in the preview",
        map: Map::Tenxer,
        command: TENXER_MEDIA_MUTE,
        candidate: Binding::alt(Key::m),
        table: &[],
        policy: OFF,
        kind: Kind::ChooserMnemonic,
        found: false,
    },
    Row {
        case: "a Go step on a pin's digit",
        map: Map::Tenxer,
        command: GO_HOME,
        candidate: Binding::plain(Key::_1),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::Digits(PIN_NUMBERS)),
        found: true,
    },
    Row {
        case: "a Go step on 0, which no pin uses",
        map: Map::Tenxer,
        command: GO_HOME,
        candidate: Binding::plain(Key::_0),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::Digits(PIN_NUMBERS)),
        found: false,
    },
    Row {
        case: "an Action step on an action's digit",
        map: Map::Tenxer,
        command: ACTIONS_TERMINAL,
        candidate: Binding::plain(Key::_0),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::Digits(ACTIONS_NUMBERS)),
        found: true,
    },
    Row {
        case: "a digit step in a chord without digits",
        map: Map::Tenxer,
        command: COPY_AS_NAME,
        candidate: Binding::plain(Key::_1),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::Digits(PIN_NUMBERS)),
        found: false,
    },
    Row {
        case: "Ctrl+Shift with a digit, which selects a tab",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::ctrl_shift(Key::_2),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::Digits(TAB_NUMBERS)),
        found: true,
    },
    Row {
        case: "a preview command on a tab digit",
        map: Map::Default,
        command: FIND_IN_PREVIEW,
        candidate: Binding::ctrl_shift(Key::_2),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::Digits(TAB_NUMBERS)),
        found: true,
    },
    Row {
        case: "Ctrl with a digit, whose tolerated Shift the tab strip takes first",
        map: Map::Default,
        command: PREVIEW,
        candidate: Binding::ctrl(Key::_7),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::Digits(TAB_NUMBERS)),
        found: false,
    },
    Row {
        case: "Backspace in the trash confirmation",
        map: Map::Tenxer,
        command: TENXER_DELETE_CONFIRM,
        candidate: Binding::plain(Key::BackSpace),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::Backspace),
        found: false,
    },
    Row {
        case: "Space in the default file list, which only preview holds",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::plain(Key::space),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::SelectionToggle),
        found: false,
    },
    Row {
        case: "Space, which plays and pauses a media preview",
        map: Map::Tenxer,
        command: TENXER_MEDIA_MUTE,
        candidate: Binding::plain(Key::space),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::MediaKeys),
        found: true,
    },
    Row {
        case: "Shift+Space in a media preview",
        map: Map::Tenxer,
        command: TENXER_MEDIA_MUTE,
        candidate: Binding::shift(Key::space),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::MediaKeys),
        found: false,
    },
    Row {
        case: "a seek on Home, which a media preview reads as seeking to the start",
        map: Map::Tenxer,
        command: MEDIA_SEEK_END,
        candidate: Binding::plain(Key::Home),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::MediaKeys),
        found: true,
    },
    Row {
        case: "Backspace, which opens the parent folder",
        map: Map::Default,
        command: HISTORY_BACK,
        candidate: Binding::shift(Key::BackSpace),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::Backspace),
        found: true,
    },
    Row {
        case: "Ctrl+Backspace, which no fixed role takes",
        map: Map::Default,
        command: HISTORY_BACK,
        candidate: Binding::ctrl(Key::BackSpace),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::Backspace),
        found: false,
    },
    Row {
        case: "Space, which selects the 10xer cursor's item",
        map: Map::Tenxer,
        command: RENAME,
        candidate: Binding::plain(Key::space),
        table: &[],
        policy: OFF,
        kind: Kind::FixedKey(Fixed::SelectionToggle),
        found: true,
    },
    Row {
        case: "an Escape alternate on an arrow, which modifier-tolerant Escape matchers read",
        map: Map::Default,
        command: ESCAPE,
        candidate: Binding::plain(Key::Down),
        table: &[],
        policy: OFF,
        kind: Kind::Duplicate(DOWN),
        found: true,
    },
    Row {
        case: "an Escape alternate on an arrow still fires",
        map: Map::Default,
        command: ESCAPE,
        candidate: Binding::plain(Key::Down),
        table: &[],
        policy: OFF,
        kind: Kind::NeverMatches,
        found: false,
    },
    Row {
        case: "a typed key for a command the 10xer stage passes on, which the listing keeps",
        map: Map::Tenxer,
        command: NEW_FOLDER,
        candidate: Binding::plain(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::NeverMatches,
        found: true,
    },
    Row {
        case: "a typed key for a command the 10xer stage reads",
        map: Map::Tenxer,
        command: PREVIEW,
        candidate: Binding::plain(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::NeverMatches,
        found: false,
    },
    Row {
        case: "a saved key of a command the map has no path to",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::ctrl(Key::e),
        table: &[(YANK, &[Binding::ctrl(Key::e)])],
        policy: OFF,
        kind: Kind::Duplicate(YANK),
        found: false,
    },
    Row {
        case: "a 10xer verb in the default map",
        map: Map::Default,
        command: YANK,
        candidate: Binding::ctrl(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::NoPath,
        found: true,
    },
    Row {
        case: "a Go step in the default map, which has no chords",
        map: Map::Default,
        command: GO_HOME,
        candidate: Binding::ctrl(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::NoPath,
        found: true,
    },
    Row {
        case: "the type-to-search filter key in 10xer",
        map: Map::Tenxer,
        command: FILTER_SLASH,
        candidate: Binding::ctrl(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::NoPath,
        found: true,
    },
    Row {
        case: "a 10xer verb the direct stage runs in the default map",
        map: Map::Default,
        command: OPEN_WITH,
        candidate: Binding::ctrl(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::NoPath,
        found: false,
    },
    Row {
        case: "a default-map verb the direct stage runs in 10xer",
        map: Map::Tenxer,
        command: DUPLICATE,
        candidate: Binding::plain(Key::w),
        table: &[],
        policy: OFF,
        kind: Kind::NoPath,
        found: false,
    },
    Row {
        case: "a typed key on a direct 10xer command, which the listing passes on",
        map: Map::Tenxer,
        command: DUPLICATE,
        candidate: Binding::plain(Key::w),
        table: &[],
        policy: OFF,
        kind: Kind::NeverMatches,
        found: false,
    },
    Row {
        case: "a modified key on a default-map command the 10xer listing still reads",
        map: Map::Tenxer,
        command: COPY_PATH,
        candidate: Binding::ctrl(Key::e),
        table: &[],
        policy: OFF,
        kind: Kind::NoPath,
        found: false,
    },
    Row {
        case: "a typed key on that command, which the 10xer listing keeps",
        map: Map::Tenxer,
        command: COPY_PATH,
        candidate: Binding::plain(Key::w),
        table: &[],
        policy: OFF,
        kind: Kind::NeverMatches,
        found: true,
    },
    Row {
        case: "Shift with a symbol it types",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::shift(Key::exclam),
        table: &[],
        policy: OFF,
        kind: Kind::NeverMatches,
        found: true,
    },
    Row {
        case: "a chord trigger in the default map, which has no chords",
        map: Map::Default,
        command: GO,
        candidate: Binding::plain(Key::g),
        table: &[],
        policy: OFF,
        kind: Kind::NeverMatches,
        found: true,
    },
    Row {
        case: "a typed key for the folder accept, which the 10xer listing keeps",
        map: Map::Tenxer,
        command: CHOOSE_FOLDER,
        candidate: Binding::plain(Key::z),
        table: &[],
        policy: OFF,
        kind: Kind::NeverMatches,
        found: true,
    },
    Row {
        case: "a canonical key",
        map: Map::Default,
        command: ESCAPE,
        candidate: Binding::plain(Key::Escape),
        table: &[],
        policy: OFF,
        kind: Kind::NeverMatches,
        found: false,
    },
    Row {
        case: "Shift with a digit, which keeps its position",
        map: Map::Default,
        command: RENAME,
        candidate: Binding::shift(Key::_1),
        table: &[],
        policy: OFF,
        kind: Kind::NeverMatches,
        found: false,
    },
];

fn table(entries: &[(Command, &[Binding])], map: Map) -> Keymap {
    entries
        .iter()
        .fold(Keymap::defaults(), |keymap, (command, bindings)| {
            keymap.with(map, *command, bindings.to_vec())
        })
}

#[test]
fn each_kind_has_a_failing_and_a_passing_row() {
    let mut checked = 0;
    for row in ROWS {
        let findings = check(
            &table(row.table, row.map),
            row.map,
            row.command,
            row.candidate,
            row.policy,
        );
        if row.found {
            let finding = findings.iter().find(|finding| finding.kind == row.kind);
            assert!(finding.is_some(), "{}: {findings:?}", row.case);
            let finding = finding
                .copied()
                .unwrap_or(Finding::new(row.kind, row.candidate));
            assert_eq!(finding.severity, row.kind.severity(), "{}", row.case);
            assert_eq!(finding.binding, row.candidate.normalized(), "{}", row.case);
        } else {
            assert!(
                !findings
                    .iter()
                    .any(|finding| discriminant(&finding.kind) == discriminant(&row.kind)),
                "{}: {findings:?}",
                row.case
            );
        }
        checked += 1;
    }
    for kind in [
        Kind::Duplicate(COPY),
        Kind::ShadowsChordGroup(Group::Go),
        Kind::ShiftedSortStep,
        Kind::SortStepCannotReverse,
        Kind::TypeToSearchShadowed,
        Kind::TextFieldLimit(FieldLimit::Typing),
        Kind::ContextOverlap(COPY),
        Kind::ReservedByGtk,
        Kind::InputMethodKey,
        Kind::ChooserMnemonic,
        Kind::FixedKey(Fixed::Backspace),
        Kind::NeverMatches,
        Kind::NoPath,
    ] {
        for found in [true, false] {
            assert!(
                ROWS.iter().any(|row| row.found == found
                    && discriminant(&row.kind) == discriminant(&kind)),
                "{kind:?} needs a row with found = {found}"
            );
        }
    }
    assert_eq!(checked, ROWS.len());
}

fn warning(command: Command, kind: Kind, binding: Binding) -> (Command, Finding) {
    (command, Finding::new(kind, binding))
}

/// The note each command of a by-design overlap gets.
fn overlap(
    first: Command,
    second: Command,
    binding: Binding,
) -> impl Iterator<Item = (Command, Finding)> {
    [
        warning(first, Kind::ContextOverlap(second), binding),
        warning(second, Kind::ContextOverlap(first), binding),
    ]
    .into_iter()
}

/// The shipped tables have no errors, and only these notes, in both modes of
/// type-to-search.
#[test]
fn audit_of_default_tables_is_clean() {
    let keymap = Keymap::defaults();
    let typing = Kind::TextFieldLimit(FieldLimit::Typing);
    let arrow_letters = [
        (UP, Key::k),
        (DOWN, Key::j),
        (LEFT, Key::h),
        (RIGHT, Key::l),
    ];
    let letter_notes = |kind| {
        arrow_letters
            .iter()
            .map(move |(command, key)| warning(*command, kind, Binding::plain(*key)))
    };
    let page_notes: Vec<_> = [
        (PAGE_UP, TAB_PREVIOUS, Key::Page_Up),
        (PAGE_DOWN, TAB_NEXT, Key::Page_Down),
    ]
    .into_iter()
    .flat_map(|(page, tab, key)| {
        [
            warning(page, Kind::ContextOverlap(tab), Binding::ctrl(key)),
            warning(tab, Kind::ContextOverlap(page), Binding::ctrl(key)),
        ]
    })
    .collect();
    let zoom_notes = [
        (TEXT_RESET, Kind::ContextOverlap(PDF_ZOOM_RESET)),
        (PDF_ZOOM_RESET, Kind::ContextOverlap(TEXT_RESET)),
    ]
    .map(|(command, kind)| warning(command, kind, Binding::ctrl(Key::_0)));
    let shadowed = [(COPY_PATH, Key::y), (PIN, Key::p)]
        .into_iter()
        .map(|(command, key)| warning(command, Kind::TypeToSearchShadowed, Binding::plain(key)))
        .chain(letter_notes(Kind::TypeToSearchShadowed));
    let notes = || letter_notes(typing).chain(zoom_notes);
    // A focused preview reads its own keys before the window's.
    let default_preview_notes = || {
        overlap(FILTER, FIND_IN_PREVIEW, Binding::ctrl(Key::f)).chain(overlap(
            OPEN_IN_NEW_WINDOW,
            PREVIOUS_MATCH,
            Binding::shift(Key::Return),
        ))
    };
    let tenxer_preview_notes = || {
        [
            (FIND, FIND_IN_PREVIEW),
            (LAST_ITEM, MEDIA_SEEK_END),
            (THUMBNAILS_LARGER, MEDIA_VOLUME_STEP_UP),
            (THUMBNAILS_SMALLER, MEDIA_VOLUME_STEP_DOWN),
            (OPEN_IN_NEW_WINDOW, PREVIOUS_MATCH),
        ]
        .into_iter()
        .flat_map(|(window, preview)| {
            window
                .defaults(Map::Tenxer)
                .iter()
                .filter(move |binding| preview.defaults(Map::Tenxer).contains(binding))
                .flat_map(move |binding| overlap(window, preview, *binding))
        })
    };
    let expected = [
        (
            Map::Default,
            OFF,
            notes().chain(default_preview_notes()).collect::<Vec<_>>(),
        ),
        (
            Map::Default,
            ON,
            notes()
                .chain(default_preview_notes())
                .chain(shadowed)
                .collect(),
        ),
        (
            Map::Tenxer,
            OFF,
            notes()
                .chain(page_notes.clone())
                .chain(tenxer_preview_notes())
                .collect(),
        ),
        (
            Map::Tenxer,
            ON,
            notes()
                .chain(page_notes)
                .chain(tenxer_preview_notes())
                .collect(),
        ),
    ];
    for (map, policy, notes) in expected {
        let mut found = audit(&keymap, map, policy);
        let key = |(command, finding): &(Command, Finding)| {
            format!("{} {:?} {:?}", command.id(), finding.kind, finding.binding)
        };
        found.sort_by_key(key);
        let mut notes = notes;
        notes.sort_by_key(key);
        assert!(
            found
                .iter()
                .all(|(_, finding)| finding.severity == Severity::Warning),
            "{map:?} {policy:?}: {found:#?}"
        );
        assert_eq!(found, notes, "{map:?} {policy:?}");
    }
}

#[test]
fn audit_reports_each_side_of_a_conflict() {
    let keymap = Keymap::defaults().with(Map::Default, RENAME, vec![Binding::ctrl(Key::c)]);
    let found = audit(&keymap, Map::Default, OFF);
    for (command, other) in [(RENAME, COPY), (COPY, RENAME)] {
        assert!(
            found.contains(&(
                command,
                Finding::new(Kind::Duplicate(other), Binding::ctrl(Key::c))
            )),
            "{command:?}: {found:#?}"
        );
    }
}
