// SPDX-License-Identifier: MIT

//! Checks keymap entries against their table: keys another command already
//! takes there, keys a chord, GTK, a text field or an input method claims, and
//! keys that never fire. Findings are data for Settings and the load-time log
//! to render. See `docs/keyboard-shortcuts.md`.

use gtk::gdk::{Key, ModifierType as Modifiers};

use super::{
    Binding, CUSTOM_ACTIONS, Command, DIRECT_IN_TENXER, Digits, Keymap, Map, SHORTCUT_MODIFIERS,
    alias,
    bindings::{
        ACTIONS_NUMBERS, ARROW_SCOPE, BACKSPACE, CHOOSE_FOLDER, COMPLETE, CONTEXT_MENU, COPY,
        COPY_PATH, CUT, DELETE_PERMANENTLY, DOWN, END_KEY, ENTER, ESCAPE, EXTEND_DOWN, EXTEND_LEFT,
        EXTEND_PAGE_DOWN, EXTEND_PAGE_UP, EXTEND_RIGHT, EXTEND_TO_FIRST, EXTEND_TO_LAST, EXTEND_UP,
        FILTER, FILTER_SLASH, FIND_IN_PREVIEW, FOCUS_NEXT, FOCUS_PREVIOUS, GO_PROMPT, HIDDEN_FILES,
        HISTORY_BACK, HISTORY_FORWARD, HOME_FOLDER, HOME_KEY, JUMP_TO_FOLDER, LEFT, LOCATION,
        MEDIA_MUTE, MEDIA_NEXT, MEDIA_PLAY_PAUSE, MEDIA_PREVIOUS, MEDIA_SEEK_BACK,
        MEDIA_SEEK_BACK_PAGE, MEDIA_SEEK_END, MEDIA_SEEK_FORWARD, MEDIA_SEEK_FORWARD_PAGE,
        MEDIA_SEEK_START, MEDIA_VOLUME_DOWN, MEDIA_VOLUME_STEP_DOWN, MEDIA_VOLUME_STEP_UP,
        MEDIA_VOLUME_UP, MOVE_TO_TRASH, NEW_FOLDER, NEW_FOLDER_WITH_SELECTION,
        OPEN_CONTAINING_FOLDER, OPEN_IN_NEW_TAB, OPEN_IN_NEW_WINDOW, PAGE_DOWN, PAGE_DOWN_KEY,
        PAGE_UP, PAGE_UP_KEY, PARENT_FOLDER, PASTE, PDF_ZOOM_RESET, PIN_NUMBERS, PREVIEW,
        PREVIOUS_MATCH, PROPERTIES, REDO, REFERENCE, REFERENCE_CATEGORIES, REFERENCE_LIST,
        REFERENCE_SEARCH, REFRESH, RENAME, RIGHT, SCROLL_PREVIEW_DOWN, SCROLL_PREVIEW_UP, SEARCH,
        SELECT_ALL, SELECTION_TOGGLE, SETTINGS, SIDEBAR, SIDEBAR_FOCUS, SPACE, TAB_CHORD_NUMBERS,
        TAB_CLOSE, TAB_MOVE_LEFT, TAB_MOVE_RIGHT, TAB_NEW, TAB_NEXT, TAB_NUMBERS, TAB_PREVIOUS,
        TAB_REOPEN, TENXER_DELETE_CONFIRM, TENXER_MEDIA_MUTE, TENXER_MEDIA_NEXT,
        TENXER_MEDIA_PREVIOUS, TENXER_MODE, TERMINAL, TEXT_LARGER, TEXT_RESET, TEXT_SMALLER,
        THUMBNAILS_LARGER, THUMBNAILS_RESET, THUMBNAILS_SMALLER, UNDO, UP, VIEW_COLUMNS,
        VIEW_ICONS, VIEW_LIST,
    },
    command::{Group, Kind as CommandKind, registry},
    digit_value, direct_commands,
};

/// What the checker cannot read from the table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Policy {
    /// Type-to-search is on, so the default map's file list types plain
    /// printable keys.
    pub(crate) type_to_search: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Severity {
    /// The key never fires where it is bound, or fires and stops another key
    /// or a fixed role from working; Settings refuses it.
    Error,
    /// The key is saved but works only in some places.
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Another command the same surface matches holds the key. Chord steps
    /// count only against steps of their group and the keys an armed chord
    /// reads before its steps.
    Duplicate(Command),
    /// A top-level key equal to a 10xer chord trigger, so one of them never
    /// acts.
    ShadowsChordGroup(Group),
    /// A Sort step with Shift, which reverses a step.
    ShiftedSortStep,
    /// A Sort step on a digit or a symbol that Shift types, which Shift cannot
    /// reverse.
    SortStepCannotReverse,
    /// A plain printable key in the default map while type-to-search types it.
    TypeToSearchShadowed,
    /// A key that text fields keep from a command that also acts in them: a
    /// plain printable key, or for Escape, Enter, Tab and the arrows another
    /// key a field binds itself.
    TextFieldLimit(FieldLimit),
    /// Both commands take the key in some state, and the stage that sees the
    /// key first decides.
    ContextOverlap(Command),
    /// A key GTK list views own: the selection toggle and Shift extension.
    ReservedByGtk,
    /// A key an input method needs while a text field has focus.
    InputMethodKey,
    /// Alt with a letter, which the file chooser's accept button may take as
    /// its mnemonic.
    ChooserMnemonic,
    /// A key a fixed role holds where the command acts, so one of them never
    /// does.
    FixedKey(Fixed),
    /// No press reaches the command with the binding where it acts.
    NeverMatches,
    /// Nothing runs the command in this map, whatever its key.
    NoPath,
}

/// A role that keeps its keys whatever the keymap says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fixed {
    /// Space selects the item under the 10xer cursor.
    SelectionToggle,
    /// Space plays and pauses a media preview, and the page keys, Home and
    /// End seek it, before its later keys.
    MediaKeys,
    /// Backspace opens the parent folder and deletes in text fields.
    Backspace,
    /// A digit range: pins after Go, actions after the Action chord, tabs
    /// after the Tabs chord or with Ctrl+Shift.
    Digits(Digits),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FieldLimit {
    /// A plain printable key, which a focused text field or popover types.
    Typing,
    /// A key text fields bind themselves, such as Ctrl+A.
    FieldKey,
}

impl Kind {
    pub(crate) const fn severity(self) -> Severity {
        match self {
            Self::Duplicate(_)
            | Self::ShadowsChordGroup(_)
            | Self::ShiftedSortStep
            | Self::ReservedByGtk
            | Self::InputMethodKey
            | Self::FixedKey(_)
            | Self::NeverMatches
            | Self::NoPath => Severity::Error,
            Self::SortStepCannotReverse
            | Self::TypeToSearchShadowed
            | Self::TextFieldLimit(_)
            | Self::ContextOverlap(_)
            | Self::ChooserMnemonic => Severity::Warning,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) severity: Severity,
    pub(crate) kind: Kind,
    /// The alternate the finding is about.
    pub(crate) binding: Binding,
}

impl Finding {
    fn new(kind: Kind, binding: Binding) -> Self {
        Self {
            severity: kind.severity(),
            kind,
            binding,
        }
    }

    /// Whether the binding never fires in the state checked, so a row shows
    /// it inactive.
    pub(crate) fn inactive(self) -> bool {
        matches!(
            self.kind,
            Kind::TypeToSearchShadowed | Kind::NeverMatches | Kind::NoPath
        )
    }
}

/// Findings for adding `candidate`, in its saved form, to `map`'s entry for
/// `command`.
pub(crate) fn check(
    keymap: &Keymap,
    map: Map,
    command: Command,
    candidate: Binding,
    policy: Policy,
) -> Vec<Finding> {
    let candidate = candidate.normalized();
    if no_path(command, map) {
        return vec![Finding::new(Kind::NoPath, candidate)];
    }
    let shipped = command.defaults(map).contains(&candidate);
    let mut findings = Vec::new();
    let mut add = |kind| {
        let finding = Finding::new(kind, candidate);
        if !findings.contains(&finding) {
            findings.push(finding);
        }
    };
    let mine = Side {
        command,
        place: Place::of(command, map),
        binding: candidate,
        map,
    };
    if never_matches(mine) {
        add(Kind::NeverMatches);
    }
    compare_with_table(keymap, mine, &mut add);
    if let Some(fixed) = fixed_key(mine) {
        add(Kind::FixedKey(fixed));
    }
    let scope = match mine.place {
        Place::Top(scope) => scope,
        Place::Step(Group::Sort) => {
            if let Some(kind) = sort_step(candidate) {
                add(kind);
            }
            return findings;
        }
        Place::Step(_) | Place::Inert => return findings,
    };
    let in_list_views = matches!(scope, Scope::Window | Scope::Chooser);
    let in_text_fields = acts_in_text_fields(command, map);
    // Focused fields keep plain printable keys from structural aliases and
    // from keys a user adds; shipped keys of other commands still act there.
    if candidate.is_typed() && in_text_fields && (mine.is_alias() || !shipped) {
        add(Kind::TextFieldLimit(FieldLimit::Typing));
    } else if mine.is_alias() && TEXT_FIELD_KEYS.iter().any(|key| overlaps(*key, candidate)) {
        add(Kind::TextFieldLimit(FieldLimit::FieldKey));
    }
    // Added plain printable keys yield to type-to-search on every surface.
    if map == Map::Default
        && policy.type_to_search
        && candidate.is_typed()
        && (!shipped
            || (scope == Scope::Window && command != FILTER_SLASH && candidate.key() != Key::space))
    {
        add(Kind::TypeToSearchShadowed);
    }
    if in_list_views && GTK_OWNED.iter().any(|key| overlaps(*key, candidate)) {
        add(Kind::ReservedByGtk);
    }
    if in_text_fields
        && (IM_SWITCH_KEYS.iter().any(|key| overlaps(*key, candidate))
            || (shipped && starts_a_syllable(candidate)))
    {
        add(Kind::InputMethodKey);
    }
    if in_list_views && is_mnemonic(candidate) {
        add(Kind::ChooserMnemonic);
    }
    findings
}

/// Every finding for every binding of `map`'s table.
pub(crate) fn audit(keymap: &Keymap, map: Map, policy: Policy) -> Vec<(Command, Finding)> {
    audit_where(keymap, map, policy, |_, _| true)
}

/// The findings for the keys `map`'s entries hold beyond their shipped ones,
/// as a hand-edited settings file adds them.
pub(crate) fn audit_added(keymap: &Keymap, map: Map, policy: Policy) -> Vec<(Command, Finding)> {
    audit_where(keymap, map, policy, |command, binding| {
        !command.defaults(map).contains(&binding)
    })
}

fn audit_where(
    keymap: &Keymap,
    map: Map,
    policy: Policy,
    include: impl Fn(Command, Binding) -> bool,
) -> Vec<(Command, Finding)> {
    let mut findings = Vec::new();
    for command in registry() {
        for binding in keymap.bindings(map, command) {
            if include(command, *binding) {
                findings.extend(
                    check(keymap, map, command, *binding, policy)
                        .into_iter()
                        .map(|finding| (command, finding)),
                );
            }
        }
    }
    findings
}

/// Whether `binding` in `map`'s entry for `command` never fires, so labels
/// leave it out: no press reaches the command with it, or, while `typing`
/// (type-to-search is active), it is a plain printable key a user added to the
/// default map.
pub(super) fn never_fires(map: Map, command: Command, binding: Binding, typing: bool) -> bool {
    let binding = binding.normalized();
    let side = Side {
        command,
        place: Place::of(command, map),
        binding,
        map,
    };
    no_path(command, map)
        || never_matches(side)
        || (typing
            && map == Map::Default
            && binding.is_typed()
            && !command.defaults(map).contains(&binding))
}

/// Which of two commands on one key acts, for a [`Kind::Duplicate`] finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The other command reads the key first; this binding never fires.
    Loses,
    /// This binding reads the key first, and the other command stops working.
    Wins,
    /// The stage order decides, which the checker does not model.
    Either,
}

/// The outcome of `command` and `other` both holding `binding` in `map`.
pub(crate) fn duplicate_outcome(
    map: Map,
    command: Command,
    binding: Binding,
    other: Command,
) -> Outcome {
    let binding = binding.normalized();
    let side = |command| Side {
        command,
        place: Place::of(command, map),
        binding,
        map,
    };
    let (mine, theirs) = (side(command), side(other));
    match (mine.place, theirs.place) {
        (Place::Step(_), Place::Top(_)) if reads_before_steps(other) => Outcome::Loses,
        (Place::Top(_), Place::Step(_)) if reads_before_steps(command) => Outcome::Wins,
        (Place::Top(_), Place::Top(_))
            if media_reads_first(theirs, command) || runs_first(other, command, map) =>
        {
            Outcome::Loses
        }
        (Place::Top(_), Place::Top(_))
            if media_reads_first(mine, other) || runs_first(command, other, map) =>
        {
            Outcome::Wins
        }
        _ => Outcome::Either,
    }
}

/// Where a command's bindings are matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// Only while its 10xer chord is armed.
    Step(Group),
    Top(Scope),
    /// A chord trigger in the default map, which has no chords.
    Inert,
}

impl Place {
    fn of(command: Command, map: Map) -> Self {
        match command.kind() {
            CommandKind::Step(group) if map == Map::Tenxer => Self::Step(group),
            CommandKind::Trigger(_) if map == Map::Default => Self::Inert,
            _ => Self::Top(
                SCOPES
                    .iter()
                    .find(|(scoped, _)| *scoped == command)
                    .map_or(Scope::Window, |(_, scope)| *scope),
            ),
        }
    }
}

/// The surface that matches a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    /// The window dispatcher, which also serves the file chooser's listing.
    Window,
    Preview,
    Reference,
    Search,
    Chooser,
    Completion,
    TrashDialog,
}

/// Commands one surface matches; every other command is the window's.
const SCOPES: &[(Command, Scope)] = &[
    (TENXER_MEDIA_MUTE, Scope::Preview),
    (TENXER_MEDIA_PREVIOUS, Scope::Preview),
    (TENXER_MEDIA_NEXT, Scope::Preview),
    (MEDIA_SEEK_BACK_PAGE, Scope::Preview),
    (MEDIA_SEEK_FORWARD_PAGE, Scope::Preview),
    (MEDIA_SEEK_START, Scope::Preview),
    (MEDIA_SEEK_END, Scope::Preview),
    (MEDIA_VOLUME_STEP_UP, Scope::Preview),
    (MEDIA_VOLUME_STEP_DOWN, Scope::Preview),
    (FIND_IN_PREVIEW, Scope::Preview),
    (PDF_ZOOM_RESET, Scope::Preview),
    (PREVIOUS_MATCH, Scope::Preview),
    (REFERENCE_CATEGORIES, Scope::Reference),
    (REFERENCE_SEARCH, Scope::Reference),
    (REFERENCE_LIST, Scope::Reference),
    (OPEN_CONTAINING_FOLDER, Scope::Search),
    (CHOOSE_FOLDER, Scope::Chooser),
    (COMPLETE, Scope::Completion),
    (TENXER_DELETE_CONFIRM, Scope::TrashDialog),
];

/// Preview commands only a media preview runs.
const MEDIA: [Command; 9] = [
    TENXER_MEDIA_MUTE,
    TENXER_MEDIA_PREVIOUS,
    TENXER_MEDIA_NEXT,
    MEDIA_SEEK_BACK_PAGE,
    MEDIA_SEEK_FORWARD_PAGE,
    MEDIA_SEEK_START,
    MEDIA_SEEK_END,
    MEDIA_VOLUME_STEP_UP,
    MEDIA_VOLUME_STEP_DOWN,
];

/// Preview commands the 10xer listing also runs while a media preview is
/// open, passing the key on otherwise.
const LISTING_TOO: [Command; 2] = [TENXER_MEDIA_PREVIOUS, TENXER_MEDIA_NEXT];

/// Text size runs before every surface's own keys and before an armed chord,
/// after only a focused document's zoom reset.
const EVERY_SURFACE: [Command; 3] = [TEXT_LARGER, TEXT_SMALLER, TEXT_RESET];

/// What a tabbed window's tab strip takes before its panes see the key.
const TAB_STRIP: [Command; 7] = [
    TAB_NEW,
    TAB_CLOSE,
    TAB_REOPEN,
    TAB_NEXT,
    TAB_PREVIOUS,
    TAB_MOVE_LEFT,
    TAB_MOVE_RIGHT,
];

/// Whether the tab strip reads keys before `place`: every surface but the
/// file chooser, which has no tabs, and modal dialogs, which block the strip.
fn under_tab_strip(place: Place) -> bool {
    !matches!(
        place,
        Place::Inert | Place::Top(Scope::Chooser | Scope::Search | Scope::TrashDialog)
    )
}

/// What the file chooser's own handler reads before `choose_folder`, each
/// stopping there.
const CHOOSER_FIRST: &[(Command, Tables)] = &[
    (TENXER_MODE, Tables::Both),
    (CONTEXT_MENU, Tables::Both),
    (ESCAPE, Tables::Both),
    (RENAME, Tables::Default),
    (PROPERTIES, Tables::Both),
    (FILTER, Tables::Default),
    (LOCATION, Tables::Both),
    (SIDEBAR_FOCUS, Tables::Both),
    (SIDEBAR, Tables::Both),
];

/// What else the chooser's handler reads: before `choose_folder` only in some
/// states, or after it, so the folder accept takes their key in folder
/// requests. In 10xer mode the 10xer stage also reads its table first.
const CHOOSER_SHARED: &[(Command, Tables)] = &[
    (THUMBNAILS_LARGER, Tables::Default),
    (THUMBNAILS_SMALLER, Tables::Default),
    (THUMBNAILS_RESET, Tables::Both),
    (PREVIEW, Tables::Default),
    (NEW_FOLDER, Tables::Both),
    (SELECT_ALL, Tables::Both),
    (REFRESH, Tables::Both),
    (HIDDEN_FILES, Tables::Default),
    (MOVE_TO_TRASH, Tables::Default),
    (DELETE_PERMANENTLY, Tables::Default),
    (VIEW_COLUMNS, Tables::Both),
    (VIEW_ICONS, Tables::Both),
    (VIEW_LIST, Tables::Both),
    (HISTORY_BACK, Tables::Default),
    (HISTORY_FORWARD, Tables::Default),
    (PARENT_FOLDER, Tables::Both),
    (HOME_FOLDER, Tables::Both),
];

/// Listing commands the 10xer stage passes on in a chooser, which has no
/// tabs or windows to open and runs neither the item commands nor the direct
/// ones.
fn chooser_declines(command: Command) -> bool {
    [OPEN_IN_NEW_TAB, OPEN_IN_NEW_WINDOW, COPY_PATH].contains(&command)
        || DIRECT_IN_TENXER.contains(&command)
}

/// Commands the 10xer stage passes on in a window but refuses in a chooser,
/// taking their keys there.
const CHOOSER_REFUSES: [Command; 3] = [COPY, CUT, PASTE];

/// What else the 10xer stage leaves to later stages: the window's default-map
/// stages, or the file chooser's handler. It claims every other key of its
/// table first.
const TENXER_PASSES: &[Command] = &[
    LOCATION,
    SIDEBAR,
    SIDEBAR_FOCUS,
    CONTEXT_MENU,
    PROPERTIES,
    COPY,
    CUT,
    PASTE,
    HOME_FOLDER,
    COPY_PATH,
    VIEW_COLUMNS,
    VIEW_ICONS,
    VIEW_LIST,
    THUMBNAILS_LARGER,
    THUMBNAILS_SMALLER,
    THUMBNAILS_RESET,
    NEW_FOLDER,
    NEW_FOLDER_WITH_SELECTION,
    REFRESH,
    SEARCH,
    SETTINGS,
    TERMINAL,
    UNDO,
    REDO,
    FOCUS_NEXT,
    FOCUS_PREVIOUS,
    TAB_CLOSE,
    TAB_REOPEN,
    TAB_NEXT,
    TAB_PREVIOUS,
    TAB_MOVE_LEFT,
    TAB_MOVE_RIGHT,
    MEDIA_PLAY_PAUSE,
    MEDIA_SEEK_BACK,
    MEDIA_SEEK_FORWARD,
    MEDIA_VOLUME_UP,
    MEDIA_VOLUME_DOWN,
    MEDIA_MUTE,
    MEDIA_PREVIOUS,
    MEDIA_NEXT,
];

/// Pairs that share a key by design, each acting where the other does not.
/// Ctrl with the page keys pages where no tab strip takes them first.
const KNOWN_OVERLAPS: &[(Command, Command)] = &[(PAGE_UP, TAB_PREVIOUS), (PAGE_DOWN, TAB_NEXT)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tables {
    Both,
    /// Stages that read the default table in both modes.
    Default,
}

impl Tables {
    fn include(self, map: Map) -> bool {
        self == Self::Both || map == Map::Default
    }
}

const SHIFT: Modifiers = Modifiers::SHIFT_MASK;
const SUPER: Modifiers = Modifiers::SUPER_MASK;
const SHIFT_SUPER: Modifiers = SHIFT.union(SUPER);
const DELETE_HELD: Modifiers = Modifiers::CONTROL_MASK
    .union(Modifiers::ALT_MASK)
    .union(SUPER);

/// Modifiers some surface also accepts held with a command's own, from the
/// `ignoring(..)` matchers that run it. Structural alternates tolerate any.
const TOLERATED: &[(Command, Modifiers, Tables)] = &[
    (REFRESH, SHORTCUT_MODIFIERS, Tables::Both),
    (REFERENCE, SHORTCUT_MODIFIERS, Tables::Default),
    (PROPERTIES, SUPER, Tables::Both),
    (CHOOSE_FOLDER, SUPER, Tables::Both),
    (PREVIEW, SHIFT_SUPER, Tables::Default),
    (HISTORY_BACK, SHIFT_SUPER, Tables::Default),
    (HISTORY_FORWARD, SHIFT_SUPER, Tables::Default),
    (PARENT_FOLDER, SHIFT_SUPER, Tables::Both),
    (HOME_FOLDER, SHIFT_SUPER, Tables::Both),
    (OPEN_IN_NEW_TAB, SHIFT, Tables::Both),
    (PAGE_UP, SUPER, Tables::Default),
    (PAGE_DOWN, SUPER, Tables::Default),
    (GO_PROMPT, SHIFT, Tables::Both),
    (MOVE_TO_TRASH, DELETE_HELD, Tables::Default),
    (DELETE_PERMANENTLY, DELETE_HELD, Tables::Default),
    (COMPLETE, SHIFT, Tables::Both),
];

/// Commands a stage matches while a text field has focus.
const IN_TEXT_FIELDS: &[Command] = &[
    ESCAPE,
    ENTER,
    FOCUS_NEXT,
    FOCUS_PREVIOUS,
    UP,
    DOWN,
    LEFT,
    RIGHT,
    COMPLETE,
    TEXT_LARGER,
    TEXT_SMALLER,
    TEXT_RESET,
    THUMBNAILS_RESET,
    TENXER_MODE,
    SETTINGS,
    TAB_NEW,
    TAB_CLOSE,
    TAB_REOPEN,
    TAB_NEXT,
    TAB_PREVIOUS,
    TAB_MOVE_LEFT,
    TAB_MOVE_RIGHT,
    REFERENCE_CATEGORIES,
    REFERENCE_SEARCH,
    REFERENCE_LIST,
    PREVIOUS_MATCH,
    OPEN_CONTAINING_FOLDER,
    LOCATION,
    SIDEBAR_FOCUS,
    SIDEBAR,
    NEW_FOLDER,
    NEW_FOLDER_WITH_SELECTION,
    REFRESH,
    SEARCH,
    VIEW_COLUMNS,
    VIEW_ICONS,
    VIEW_LIST,
    REFERENCE,
    HOME_FOLDER,
    JUMP_TO_FOLDER,
    ARROW_SCOPE,
];

/// Commands a text field stage matches only from the default table, most in
/// both modes. 10xer runs its terminal key from the file list and sidebar only.
const IN_DEFAULT_TEXT_FIELDS: &[Command] = &[
    TERMINAL,
    RENAME,
    FILTER,
    HIDDEN_FILES,
    THUMBNAILS_LARGER,
    THUMBNAILS_SMALLER,
    FIND_IN_PREVIEW,
    PREVIEW,
];

/// Commands a map ships no key for that a stage of that map still reads,
/// besides the direct ones.
const READ_UNBOUND: &[(Command, Map)] = &[(COPY_PATH, Map::Tenxer)];

/// Keys GTK list views act on once the dispatcher yields them.
const GTK_OWNED: [Binding; 9] = [
    SELECTION_TOGGLE,
    EXTEND_UP,
    EXTEND_DOWN,
    EXTEND_LEFT,
    EXTEND_RIGHT,
    EXTEND_PAGE_UP,
    EXTEND_PAGE_DOWN,
    EXTEND_TO_FIRST,
    EXTEND_TO_LAST,
];

/// Keys `gtk::Text` binds to its own editing actions.
const TEXT_FIELD_KEYS: [Binding; 14] = [
    Binding::ctrl(Key::a),
    Binding::ctrl_shift(Key::a),
    Binding::ctrl(Key::c),
    Binding::ctrl(Key::x),
    Binding::ctrl(Key::v),
    Binding::ctrl(Key::z),
    Binding::ctrl_shift(Key::z),
    Binding::ctrl(Key::BackSpace),
    Binding::ctrl(Key::Delete),
    Binding::ctrl(Key::period),
    Binding::ctrl(Key::semicolon),
    Binding::shift(Key::Delete),
    Binding::ctrl(Key::Insert),
    Binding::shift(Key::Insert),
];

/// fcitx5 and IBus switch input methods with these. Strata's capture stage
/// sees them before a field's GTK input method module does.
const IM_SWITCH_KEYS: [Binding; 2] = [Binding::ctrl(Key::space), Binding::new(Key::space, SUPER)];

/// One binding of one command, in the table being checked.
#[derive(Clone, Copy, Debug)]
struct Side {
    command: Command,
    place: Place,
    binding: Binding,
    map: Map,
}

impl Side {
    /// A structural command's binding other than its canonical key.
    fn is_alias(self) -> bool {
        self.command
            .canonical()
            .is_some_and(|canonical| canonical != self.binding)
    }

    /// The binding with the modifiers some surface also accepts held.
    fn widened(self) -> Binding {
        let tolerated = if self.command.canonical().is_some() {
            // `ESCAPE_ANY` reads physical Escape with any modifiers.
            if (self.is_alias() && !self.binding.is_typed())
                || (self.command == ESCAPE && self.binding.key() == Key::Escape)
            {
                SHORTCUT_MODIFIERS
            } else {
                Modifiers::empty()
            }
        } else {
            TOLERATED
                .iter()
                .filter(|(command, _, tables)| *command == self.command && tables.include(self.map))
                .fold(Modifiers::empty(), |held, (_, modifiers, _)| {
                    held.union(*modifiers)
                })
        };
        self.binding
            .ignoring(tolerated.difference(self.binding.modifiers()))
    }
}

/// Entries that cannot fire in the map, such as a saved key for a command the
/// map has no path to, hold nothing.
fn compare_with_table(keymap: &Keymap, mine: Side, add: &mut impl FnMut(Kind)) {
    for other in registry().filter(|other| *other != mine.command && !no_path(*other, mine.map)) {
        let place = Place::of(other, mine.map);
        for binding in entry(keymap, mine.map, other) {
            let theirs = Side {
                command: other,
                place,
                binding,
                map: mine.map,
            };
            if never_matches(theirs) {
                continue;
            }
            if let Some(kind) = conflict(mine, theirs) {
                add(kind);
            }
        }
    }
}

/// `command`'s bindings in `map`, with the physical Escape that always
/// matches `escape`.
fn entry(keymap: &Keymap, map: Map, command: Command) -> Vec<Binding> {
    let mut bindings = keymap.bindings(map, command).to_vec();
    if command == alias::FALLBACK
        && let Some(canonical) = command.canonical()
        && !bindings.contains(&canonical)
    {
        bindings.push(canonical);
    }
    bindings
}

fn conflict(mine: Side, theirs: Side) -> Option<Kind> {
    let exact = overlaps(mine.binding, theirs.binding);
    let widened = exact || overlaps(mine.widened(), theirs.widened());
    if !widened {
        return None;
    }
    let other = theirs.command;
    match (mine.place, theirs.place) {
        (Place::Inert, _) | (_, Place::Inert) => None,
        (Place::Step(group), Place::Step(theirs)) => (group == theirs).then_some(if exact {
            Kind::Duplicate(other)
        } else {
            Kind::ContextOverlap(other)
        }),
        (Place::Step(group), Place::Top(_)) => step_against(exact, group, other, other),
        (Place::Top(_), Place::Step(group)) => step_against(exact, group, mine.command, other),
        (Place::Top(_), Place::Top(_))
            if exact
                && (media_reads_first(mine, theirs.command)
                    || media_reads_first(theirs, mine.command)) =>
        {
            Some(Kind::Duplicate(other))
        }
        (Place::Top(scope), Place::Top(theirs_scope)) => {
            let map = mine.map;
            let same_surface = scope == theirs_scope;
            let shared = same_surface
                || also_on(mine.command, theirs_scope, map)
                || also_on(other, scope, map)
                || zoom_and_text_size(mine.command, other);
            if exact
                && !same_surface
                && (runs_first(mine.command, other, map) || runs_first(other, mine.command, map))
            {
                Some(Kind::Duplicate(other))
            } else if !shared && !mine.is_alias() && !theirs.is_alias() {
                None
            } else if !exact || !same_surface || known_overlap(mine.command, other) {
                Some(Kind::ContextOverlap(other))
            } else if let (CommandKind::Trigger(group), false) =
                (other.kind(), is_trigger(mine.command))
            {
                Some(Kind::ShadowsChordGroup(group))
            } else {
                Some(Kind::Duplicate(other))
            }
        }
    }
}

/// A chord step against a top-level command on the same key.
fn step_against(exact: bool, group: Group, top: Command, other: Command) -> Option<Kind> {
    if !exact {
        // A tolerated variant of the step, such as Shift with the Go prompt's.
        reads_before_steps(top).then_some(Kind::ContextOverlap(other))
    } else if reads_before_steps(top) {
        Some(Kind::Duplicate(other))
    } else {
        // A focused document resets its zoom over the Go chord the preview arms.
        (group == Group::Go && top == PDF_ZOOM_RESET).then_some(Kind::ContextOverlap(other))
    }
}

/// Keys read before an armed chord's steps: the tab strip, text size, the
/// reference and 10xer mode in earlier stages, then the chord's own Settings
/// and Escape.
fn reads_before_steps(command: Command) -> bool {
    TAB_STRIP.contains(&command)
        || EVERY_SURFACE.contains(&command)
        || [REFERENCE, TENXER_MODE, SETTINGS, ESCAPE].contains(&command)
}

/// Whether `first` takes the key before `second`'s surface sees it.
fn runs_first(first: Command, second: Command, map: Map) -> bool {
    let reference = Place::of(second, map) == Place::Top(Scope::Reference);
    (EVERY_SURFACE.contains(&first) && second != PDF_ZOOM_RESET)
        || (TAB_STRIP.contains(&first) && under_tab_strip(Place::of(second, map)))
        // The reference toggles, and Escape closes it, before its own keys and
        // a media preview's.
        || (first == REFERENCE && (reference || MEDIA.contains(&second)))
        || (first == ESCAPE && reference)
        || (second == CHOOSE_FOLDER
            && (listed(CHOOSER_FIRST, first, map)
                || (map == Map::Tenxer
                    && Place::of(first, map) == Place::Top(Scope::Window)
                    && (!TENXER_PASSES.contains(&first) || CHOOSER_REFUSES.contains(&first))
                    && !chooser_declines(first))))
}

/// Whether `command`, outside `scope`, also reads keys there.
fn also_on(command: Command, scope: Scope, map: Map) -> bool {
    match scope {
        Scope::Window => map == Map::Tenxer && LISTING_TOO.contains(&command),
        // The window's commands still run while the preview owns the keys.
        Scope::Preview => Place::of(command, map) == Place::Top(Scope::Window),
        Scope::Chooser => {
            listed(CHOOSER_FIRST, command, map)
                || listed(CHOOSER_SHARED, command, map)
                || (map == Map::Tenxer
                    && Place::of(command, map) == Place::Top(Scope::Window)
                    && !chooser_declines(command))
        }
        _ => false,
    }
}

/// A focused document resets its zoom before text size reads the key.
fn zoom_and_text_size(first: Command, second: Command) -> bool {
    [(first, second), (second, first)]
        .iter()
        .any(|(zoom, text)| *zoom == PDF_ZOOM_RESET && EVERY_SURFACE.contains(text))
}

fn listed(table: &[(Command, Tables)], command: Command, map: Map) -> bool {
    table
        .iter()
        .any(|(listed, tables)| *listed == command && tables.include(map))
}

fn is_trigger(command: Command) -> bool {
    matches!(command.kind(), CommandKind::Trigger(_))
}

fn known_overlap(first: Command, second: Command) -> bool {
    KNOWN_OVERLAPS
        .iter()
        .any(|pair| *pair == (first, second) || *pair == (second, first))
}

/// Whether `map` ships no key for `command` and none of its stages reads one:
/// a 10xer verb or chord step in the default map, or a default-map command in
/// 10xer, that the direct stage does not run either. The direct stage also
/// runs the default map's custom action slots.
pub(crate) fn no_path(command: Command, map: Map) -> bool {
    command.defaults(map).is_empty()
        && !is_trigger(command)
        && !direct_commands(map).contains(&command)
        && !READ_UNBOUND.contains(&(command, map))
        && !(map == Map::Default && CUSTOM_ACTIONS.contains(&command))
}

fn never_matches(side: Side) -> bool {
    // The focused 10xer listing keeps typed keys from what its stage leaves to
    // later ones, the folder accept included. Thumbnail size reads first.
    let typed_past_tenxer_stage = side.map == Map::Tenxer
        && side.binding.is_typed()
        && (side.command == CHOOSE_FOLDER
            || (TENXER_PASSES.contains(&side.command)
                && ![THUMBNAILS_LARGER, THUMBNAILS_SMALLER, THUMBNAILS_RESET]
                    .contains(&side.command)));
    side.place == Place::Inert || shift_is_consumed(side.binding) || typed_past_tenxer_stage
}

/// The fixed role holding a key where `side` acts. The tab strip reads its
/// digits first and an armed chord its steps before its digits; the other
/// roles share a surface with the command. Only the key as bound counts: a
/// variant a surface tolerates loses to the fixed role, which keeps working.
fn fixed_key(side: Side) -> Option<Fixed> {
    let binding = side.binding;
    if under_tab_strip(side.place) && in_digits(binding, TAB_NUMBERS) {
        return Some(Fixed::Digits(TAB_NUMBERS));
    }
    match side.place {
        Place::Step(group) => {
            let digits = match group {
                Group::Go => PIN_NUMBERS,
                Group::Actions => ACTIONS_NUMBERS,
                Group::Tabs => TAB_CHORD_NUMBERS,
                Group::CopyAs | Group::Sort => return None,
            };
            in_digits(binding, digits).then_some(Fixed::Digits(digits))
        }
        Place::Top(scope) => {
            if scope != Scope::TrashDialog && overlaps(binding, BACKSPACE_HELD) {
                Some(Fixed::Backspace)
            } else if scope == Scope::Window && side.map == Map::Tenxer && overlaps(binding, SPACE)
            {
                Some(Fixed::SelectionToggle)
            } else if MEDIA.contains(&side.command) && media_key_first(side) {
                Some(Fixed::MediaKeys)
            } else {
                None
            }
        }
        Place::Inert => None,
    }
}

/// Whether a media preview's own Space, page, Home or End key takes the
/// binding before the media command reads it.
fn media_key_first(side: Side) -> bool {
    let seeks_after = |commands: &[Command]| commands.contains(&side.command);
    overlaps(side.binding, SPACE)
        || ([PAGE_UP_KEY, PAGE_DOWN_KEY].contains(&side.binding)
            && seeks_after(&[
                MEDIA_SEEK_START,
                MEDIA_SEEK_END,
                MEDIA_VOLUME_STEP_UP,
                MEDIA_VOLUME_STEP_DOWN,
            ]))
        || (side.binding == HOME_KEY
            && seeks_after(&[MEDIA_SEEK_END, MEDIA_VOLUME_STEP_UP, MEDIA_VOLUME_STEP_DOWN]))
        || (side.binding == END_KEY && seeks_after(&[MEDIA_VOLUME_STEP_UP, MEDIA_VOLUME_STEP_DOWN]))
}

/// Whether `first` takes the binding before a 10xer media preview's `media`
/// command: the preview's own exits and scrolling, then volume on the up and
/// down keys and seeking on left and right, as pressed rather than typed.
fn media_reads_first(first: Side, media: Command) -> bool {
    if first.map != Map::Tenxer || !MEDIA.contains(&media) {
        return false;
    }
    let typed = first.binding.is_typed();
    let seeks = !LISTED_BEFORE_SEEKING.contains(&media);
    if [UP, DOWN].contains(&first.command) {
        !typed
    } else if first.command == LEFT {
        first.is_alias() || seeks
    } else if first.command == RIGHT {
        !typed && seeks
    } else {
        [
            ESCAPE,
            FOCUS_NEXT,
            FOCUS_PREVIOUS,
            PREVIEW,
            SCROLL_PREVIEW_DOWN,
            SCROLL_PREVIEW_UP,
            TENXER_MODE,
        ]
        .contains(&first.command)
    }
}

/// Media commands a media preview reads before it seeks on left and right.
const LISTED_BEFORE_SEEKING: [Command; 3] =
    [TENXER_MEDIA_MUTE, TENXER_MEDIA_PREVIOUS, TENXER_MEDIA_NEXT];

/// Backspace's roles also accept Shift and Super held.
const BACKSPACE_HELD: Binding = BACKSPACE.ignoring(SHIFT_SUPER);

fn in_digits(binding: Binding, digits: Digits) -> bool {
    let compared = SHORTCUT_MODIFIERS
        .difference(digits.ignored())
        .difference(binding.ignored);
    digit_value(binding.key).is_some_and(|digit| digits.contains(digit))
        && binding.modifiers & compared == digits.modifiers() & compared
}

/// GDK reports the Shift that types a symbol as consumed, so a symbol never
/// matches with Shift: `<Shift>exclam` is never pressed.
fn shift_is_consumed(binding: Binding) -> bool {
    binding.modifiers().contains(SHIFT) && is_symbol(binding.key())
}

fn is_symbol(key: Key) -> bool {
    key.to_unicode().is_some_and(|character| {
        !character.is_control() && !character.is_alphanumeric() && character != ' '
    })
}

/// Shift with a Sort step reverses it, which a Shifted step, a digit that
/// already accepts Shift, or a symbol that Shift turns into another cannot.
fn sort_step(binding: Binding) -> Option<Kind> {
    if binding.modifiers().contains(SHIFT) {
        Some(Kind::ShiftedSortStep)
    } else if digit_value(binding.key()).is_some() || is_symbol(binding.key()) {
        Some(Kind::SortStepCannotReverse)
    } else {
        None
    }
}

fn acts_in_text_fields(command: Command, map: Map) -> bool {
    IN_TEXT_FIELDS.contains(&command)
        || (map == Map::Default && IN_DEFAULT_TEXT_FIELDS.contains(&command))
}

/// Zhuyin, Array30 and Boshiamy start syllables on these without Shift.
fn starts_a_syllable(binding: Binding) -> bool {
    const KEYS: [Key; 8] = [
        Key::minus,
        Key::comma,
        Key::period,
        Key::slash,
        Key::semicolon,
        Key::apostrophe,
        Key::bracketleft,
        Key::bracketright,
    ];
    binding.modifiers().is_empty()
        && (digit_value(binding.key()).is_some() || KEYS.contains(&binding.key()))
}

fn is_mnemonic(binding: Binding) -> bool {
    binding.modifiers().contains(Modifiers::ALT_MASK)
        && !binding
            .modifiers()
            .intersects(Modifiers::CONTROL_MASK | SUPER)
        && binding
            .key()
            .to_unicode()
            .is_some_and(|character| character.is_ascii_alphabetic())
}

/// Whether one press can match both bindings: the same key, or digit
/// position, with modifiers that agree wherever both compare them.
fn overlaps(first: Binding, second: Binding) -> bool {
    let same_key = match (digit_value(first.key), digit_value(second.key)) {
        (Some(first), Some(second)) => first == second,
        (None, None) => first.key == second.key,
        _ => false,
    };
    let compared = SHORTCUT_MODIFIERS
        .difference(first.ignored)
        .difference(second.ignored);
    same_key && first.modifiers & compared == second.modifiers & compared
}

#[cfg(test)]
mod tests;
