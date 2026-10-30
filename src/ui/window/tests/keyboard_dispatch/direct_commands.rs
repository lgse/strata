// SPDX-License-Identifier: MIT

//! A command bound in the map whose own stages leave it out does what its
//! key does in the other map.

use super::*;
use crate::ui::{
    keys::{
        Binding, Command, DIRECT_IN_DEFAULT, DIRECT_IN_TENXER, Keymap, Map, bindings::*, use_keymap,
    },
    tenxer_mode::Prompt,
};

#[derive(Clone, Debug, PartialEq)]
struct Seen {
    prompt: Option<Prompt>,
    feedback: String,
    modal: bool,
    selected: Vec<usize>,
    focused: String,
    sort: (SortKey, SortDirection),
    clipboard: Option<String>,
    names: Vec<String>,
    children: usize,
    pins: usize,
    window_actions: usize,
}

#[derive(Clone, Copy, Debug)]
enum Effect {
    /// A footer prompt, feedback or a dialog.
    Shown,
    Selection,
    Cursor,
    Sort,
    Clipboard,
    Copied,
    Terminal,
    Pinned,
    WindowAction,
}

impl Effect {
    /// What the press changed that this effect shows, if anything.
    fn outcome(self, before: &Seen, after: &Seen) -> Option<String> {
        let changed = |value: &dyn Fn(&Seen) -> String| {
            let value_after = value(after);
            (value(before) != value_after).then_some(value_after)
        };
        let grew = |count: fn(&Seen) -> usize| (count(after) > count(before)).then(String::new);
        match self {
            Self::Shown => {
                changed(&|seen| format!("{:?} {:?} {}", seen.prompt, seen.feedback, seen.modal))
            }
            Self::Selection => changed(&|seen| format!("{:?}", seen.selected)),
            Self::Cursor => changed(&|seen| seen.focused.clone()),
            Self::Sort => changed(&|seen| format!("{:?}", seen.sort)),
            Self::Clipboard => changed(&|seen| format!("{:?}", seen.clipboard)),
            Self::Copied => changed(&|seen| {
                format!(
                    "{:?}",
                    seen.names
                        .iter()
                        .filter(|name| !before.names.contains(name))
                        .collect::<Vec<_>>()
                )
            }),
            Self::Terminal => grew(|seen| seen.children),
            Self::Pinned => grew(|seen| seen.pins),
            Self::WindowAction => grew(|seen| seen.window_actions),
        }
    }
}

#[derive(Debug)]
struct Row {
    command: Command,
    /// The keys that run it in the map that binds it.
    existing: &'static [Binding],
    /// The item focused before the press.
    start: &'static str,
    effect: Effect,
}

const fn row(
    command: Command,
    existing: &'static [Binding],
    start: &'static str,
    effect: Effect,
) -> Row {
    Row {
        command,
        existing,
        start,
        effect,
    }
}

const SORT: Binding = Binding::plain(Key::comma);
const COPY_AS: Binding = Binding::plain(Key::c);
const ACTION: Binding = Binding::plain(Key::semicolon);
/// Holds `packed.txt`, so extracting it adds a file.
const ARCHIVE: &str = "bundle.tar";

/// Commands the default map gains, with their 10xer keys.
const DEFAULT_ROWS: &[Row] = &[
    row(OPEN_WITH, &[Binding::shift(Key::o)], "a.txt", Effect::Shown),
    row(MOVE_TO, &[Binding::shift(Key::m)], "a.txt", Effect::Shown),
    row(COPY_TO, &[Binding::shift(Key::c)], "a.txt", Effect::Shown),
    row(RESTORE, &[Binding::shift(Key::r)], "a.txt", Effect::Shown),
    row(
        ACTIONS_COMPRESS,
        &[ACTION, Binding::plain(Key::c)],
        "a.txt",
        Effect::Shown,
    ),
    row(
        ACTIONS_EXTRACT,
        &[ACTION, Binding::plain(Key::e)],
        ARCHIVE,
        Effect::Copied,
    ),
    row(
        ACTIONS_EXTRACT_TO,
        &[ACTION, Binding::shift(Key::e)],
        ARCHIVE,
        Effect::Shown,
    ),
    row(
        ACTIONS_TERMINAL,
        &[ACTION, Binding::plain(Key::t)],
        "a.txt",
        Effect::Terminal,
    ),
    row(
        SORT_BY_NAME,
        &[SORT, Binding::plain(Key::a)],
        "a.txt",
        Effect::Sort,
    ),
    row(
        SORT_BY_MODIFIED,
        &[SORT, Binding::plain(Key::m)],
        "a.txt",
        Effect::Sort,
    ),
    row(
        SORT_BY_SIZE,
        &[SORT, Binding::plain(Key::s)],
        "a.txt",
        Effect::Sort,
    ),
    row(
        SORT_BY_TYPE,
        &[SORT, Binding::plain(Key::e)],
        "a.txt",
        Effect::Sort,
    ),
    row(
        COPY_AS_PATH,
        &[COPY_AS, Binding::plain(Key::c)],
        "a.txt",
        Effect::Clipboard,
    ),
    row(
        COPY_AS_NAME,
        &[COPY_AS, Binding::plain(Key::n)],
        "a.txt",
        Effect::Clipboard,
    ),
    row(
        INVERT_SELECTION,
        &[Binding::ctrl(Key::r)],
        "a.txt",
        Effect::Selection,
    ),
    row(
        SEARCH_SUBFOLDERS,
        &[Binding::plain(Key::s)],
        "a.txt",
        Effect::Shown,
    ),
    row(
        JUMP_VISITED,
        &[Binding::plain(Key::z)],
        "a.txt",
        Effect::Shown,
    ),
    row(
        JUMP_RECENT,
        &[Binding::shift(Key::z)],
        "a.txt",
        Effect::Shown,
    ),
    row(
        HALF_PAGE_UP,
        &[Binding::ctrl(Key::u)],
        "c.txt",
        Effect::Cursor,
    ),
    row(
        HALF_PAGE_DOWN,
        &[Binding::ctrl(Key::d)],
        "a.txt",
        Effect::Cursor,
    ),
];

/// Commands 10xer gains, with their default-map keys.
const TENXER_ROWS: &[Row] = &[
    row(DUPLICATE, &[Binding::ctrl(Key::d)], "a.txt", Effect::Copied),
    row(PIN, &[Binding::plain(Key::p)], "d", Effect::Pinned),
    row(
        JUMP_TO_FOLDER,
        &[Binding::ctrl_shift(Key::k)],
        "a.txt",
        Effect::WindowAction,
    ),
    row(
        ARROW_SCOPE,
        &[Binding::ctrl(Key::backslash)],
        "a.txt",
        Effect::WindowAction,
    ),
];

struct Direct {
    fixture: KeyboardFixture,
    preferences: Rc<PreferenceManager>,
    names: Vec<String>,
    pins: Rc<Cell<usize>>,
    window_actions: Rc<Cell<usize>>,
}

impl Direct {
    fn new() -> Self {
        let fixture = KeyboardFixture::new();
        let preferences = PreferenceManager::shared();
        fixture.shortcuts.bind_preferences(&preferences);
        preferences.set_type_to_search(false);
        preferences.set_filter_include_subfolders(false);
        let directory = fixture._directory.path();
        std::fs::create_dir(directory.join("d")).expect("folder");
        let source = tempfile::tempdir().expect("archive source");
        std::fs::write(source.path().join("packed.txt"), "packed").expect("archived file");
        crate::adapters::write_compression_fixture(
            &directory.join(ARCHIVE),
            &[source.path().join("packed.txt")],
            crate::services::ArchiveFormat::Tar,
            None,
        )
        .expect("fixture archive");
        fixture.view.set_view_mode(BrowserMode::List);
        fixture.view.refresh();
        wait_until(|| {
            rendered_name(&fixture.view.widget(), "d")
                && rendered_name(&fixture.view.widget(), ARCHIVE)
        });
        let pins = Rc::new(Cell::new(0));
        let pinned = pins.clone();
        fixture.view.set_pin_handlers(
            Rc::new(move |_location, _name, done| {
                pinned.set(pinned.get() + 1);
                done(true);
            }),
            Rc::new(|_location| {}),
            Rc::new(|_| PinStatus::Available),
        );
        let window_actions = Rc::new(Cell::new(0));
        for name in ["jump-folder", "toggle-arrow-scope"] {
            let action = gio::SimpleAction::new(name, None);
            let runs = window_actions.clone();
            action.connect_activate(move |_, _| runs.set(runs.get() + 1));
            fixture.window.add_action(&action);
        }
        let names = directory_names(fixture._directory.path());
        Self {
            fixture,
            preferences,
            names,
            pins,
            window_actions,
        }
    }

    fn seen(&self) -> Seen {
        let fixture = &self.fixture;
        let browser = fixture.view.browser();
        Seen {
            prompt: fixture.shortcuts.open_prompt_kind(),
            feedback: fixture.shortcuts.feedback_text(),
            modal: modal_visible(&fixture.overlay),
            selected: fixture.selected(),
            focused: focused_name(&browser),
            sort: browser.column_preferences(0).expect("column").sort(),
            clipboard: gtk::gdk::Display::default()
                .expect("display")
                .clipboard()
                .content()
                .and_then(|content| content.value(glib::Type::STRING).ok())
                .and_then(|value| value.get::<String>().ok()),
            names: directory_names(fixture._directory.path()),
            children: child_commands().len(),
            pins: self.pins.get(),
            window_actions: self.window_actions.get(),
        }
    }

    fn enter(&self, map: Map, row: &Row) -> Seen {
        let fixture = &self.fixture;
        let browser = fixture.view.browser();
        fixture.shortcuts.cancel_chord();
        fixture.shortcuts.dismiss_prompt();
        fixture.shortcuts.dismiss_feedback();
        if modal_visible(&fixture.overlay) {
            assert!(click_class(&fixture.overlay, "action-dialog-close"));
            wait_until(|| !modal_visible(&fixture.overlay));
        }
        // A finished copy still publishes its listing; a refresh that overlaps
        // it can leave the column empty.
        let on_disk = directory_names(fixture._directory.path());
        wait_until(|| {
            browser
                .column_snapshot(0)
                .is_some_and(|column| !column.loading && column.count == on_disk.len())
        });
        for name in on_disk {
            if !self.names.contains(&name) {
                std::fs::remove_file(fixture._directory.path().join(name)).expect("copy");
            }
        }
        fixture.view.refresh();
        wait_loaded(&browser, 0);
        wait_until(|| entry_count(&browser) == self.names.len());
        let sort = match row.effect {
            Effect::Sort => (SortKey::Type, SortDirection::Descending),
            _ => (SortKey::Name, SortDirection::Ascending),
        };
        browser.set_sort(0, sort.0, sort.1);
        gtk::gdk::Display::default()
            .expect("display")
            .clipboard()
            .set_text("");
        self.preferences.set_tenxer_mode(map == Map::Tenxer);
        pump(50);
        browser.clear_active_selection();
        select_named(fixture, row.start);
        wait_until(|| focused_name(&browser) == row.start);
        self.seen()
    }

    fn press(&self, binding: Binding) {
        let modifiers = binding.modifiers();
        let key = if modifiers.contains(ModifierType::SHIFT_MASK) {
            binding.key().to_upper()
        } else {
            binding.key()
        };
        self.fixture.press(key, modifiers);
    }

    /// What `row`'s effect shows once `keys` are pressed in `map`.
    fn outcome(&self, map: Map, row: &Row, keys: &[Binding]) -> Option<String> {
        let before = self.enter(map, row);
        for key in keys {
            self.press(*key);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let outcome = row.effect.outcome(&before, &self.seen());
            // A copy or an extraction is done once its progress dialog closes.
            let settled =
                !matches!(row.effect, Effect::Copied) || !modal_visible(&self.fixture.overlay);
            if (outcome.is_some() && settled) || Instant::now() > deadline {
                return outcome;
            }
            pump(10);
        }
    }

    /// Binds `key` to each row's command in `gained` and requires it to do
    /// what the row's own keys do in the other map.
    fn agrees(&self, gained: Map, rows: &[Row], key: Binding) {
        let own = match gained {
            Map::Default => Map::Tenxer,
            Map::Tenxer => Map::Default,
        };
        for row in rows {
            let expected = self.outcome(own, row, row.existing);
            assert!(
                expected.is_some(),
                "{row:?}: the existing keys show nothing"
            );
            let _keymap = use_keymap(Keymap::defaults().with(gained, row.command, vec![key]));
            assert_eq!(
                self.outcome(gained, row, &[key]),
                expected,
                "{row:?} bound to {key:?} in {gained:?}"
            );
        }
    }
}

#[test]
fn default_map_runs_tenxer_commands_bound_to_a_key() {
    const NAME: &str = "ui::window::tests::keyboard_dispatch::direct_commands::default_map_runs_tenxer_commands_bound_to_a_key";
    // `sleep` stands in for the terminal so opening one shows up as a child process.
    crate::test_support::gtk_test_with_env(NAME, [("TERMINAL", "sleep 5")], || {
        let direct = Direct::new();
        let rows: Vec<Command> = DEFAULT_ROWS.iter().map(|row| row.command).collect();
        assert_eq!(rows, DIRECT_IN_DEFAULT, "each direct command has a row");
        direct.agrees(Map::Default, DEFAULT_ROWS, Binding::ctrl_alt(Key::e));
    });
}

/// A typed key works too: the focused 10xer listing passes it on.
#[test]
fn tenxer_runs_default_map_commands_bound_to_a_key() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::direct_commands::tenxer_runs_default_map_commands_bound_to_a_key",
        || {
            let direct = Direct::new();
            let rows: Vec<Command> = TENXER_ROWS.iter().map(|row| row.command).collect();
            assert_eq!(rows, DIRECT_IN_TENXER, "each direct command has a row");
            direct.agrees(Map::Tenxer, TENXER_ROWS, Binding::plain(Key::w));
        },
    );
}

/// With the shipped tables, the other map's keys for these commands change
/// nothing, except where a command of this map holds them.
#[test]
fn shipped_keys_never_reach_the_direct_stage() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::direct_commands::shipped_keys_never_reach_the_direct_stage",
        || {
            let direct = Direct::new();
            let keymap = Keymap::defaults();
            let free_in = |map: Map, binding: &Binding| {
                !crate::ui::keys::registry()
                    .any(|command| keymap.bindings(map, command).contains(binding))
            };
            let mut pressed = 0;
            for (map, rows, keys_from) in [
                (Map::Default, DEFAULT_ROWS, Map::Tenxer),
                (Map::Tenxer, TENXER_ROWS, Map::Default),
            ] {
                for row in rows {
                    assert!(keymap.bindings(map, row.command).is_empty(), "{row:?}");
                    for key in row.command.defaults(keys_from) {
                        if !free_in(map, key) {
                            continue;
                        }
                        let before = direct.enter(map, row);
                        direct.press(*key);
                        pump(100);
                        assert_eq!(direct.seen(), before, "{key:?} for {row:?} in {map:?}");
                        pressed += 1;
                    }
                }
            }
            assert!(pressed > 0);
        },
    );
}

/// 10xer verbs act on the item under the cursor when nothing is filled.
#[test]
fn tenxer_direct_verbs_act_on_the_cursor_item() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::direct_commands::tenxer_direct_verbs_act_on_the_cursor_item",
        || {
            let direct = Direct::new();
            let browser = direct.fixture.view.browser();
            let key = Binding::plain(Key::w);
            for row in TENXER_ROWS
                .iter()
                .filter(|row| [DUPLICATE, PIN].contains(&row.command))
            {
                let _keymap =
                    use_keymap(Keymap::defaults().with(Map::Tenxer, row.command, vec![key]));
                let before = direct.enter(Map::Tenxer, row);
                browser.clear_active_selection();
                pump(20);
                assert!(direct.fixture.selected().is_empty(), "{row:?}");
                assert_eq!(focused_name(&browser), row.start, "{row:?}");
                direct.press(key);
                wait_until(|| row.effect.outcome(&before, &direct.seen()).is_some());
            }
        },
    );
}

/// The default map's verbs act on its selection only: with the focused row
/// unselected they do nothing, as Copy does, where 10xer would take the cursor.
#[test]
fn default_map_direct_verbs_skip_an_unselected_focused_row() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::direct_commands::default_map_direct_verbs_skip_an_unselected_focused_row",
        || {
            let direct = Direct::new();
            let fixture = &direct.fixture;
            let browser = fixture.view.browser();
            let key = Binding::ctrl_alt(Key::e);
            let verbs = [
                OPEN_WITH,
                MOVE_TO,
                COPY_TO,
                ACTIONS_COMPRESS,
                ACTIONS_EXTRACT_TO,
                COPY_AS_NAME,
            ];
            for row in DEFAULT_ROWS
                .iter()
                .filter(|row| verbs.contains(&row.command))
            {
                let _keymap =
                    use_keymap(Keymap::defaults().with(Map::Default, row.command, vec![key]));
                let before = direct.enter(Map::Default, row);
                browser.clear_active_selection();
                pump(20);
                assert!(fixture.selected().is_empty(), "{row:?}");
                assert_eq!(focused_name(&browser), row.start, "{row:?}");
                direct.press(key);
                pump(100);
                let after = direct.seen();
                assert_eq!(after.prompt, None, "{row:?}");
                assert!(!after.modal, "{row:?}");
                assert_eq!(after.clipboard, before.clipboard, "{row:?}");
                assert!(
                    after.feedback.starts_with("Nothing"),
                    "{row:?}: {}",
                    after.feedback
                );
            }
        },
    );
}

/// The default map has no 10xer Escape stage, so its own Escape must end a
/// search a direct key started, which the footer names meanwhile.
#[test]
fn default_map_escape_ends_a_direct_search() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::direct_commands::default_map_escape_ends_a_direct_search",
        || {
            let direct = Direct::new();
            let fixture = &direct.fixture;
            let row = DEFAULT_ROWS
                .iter()
                .find(|row| row.command == SEARCH_SUBFOLDERS)
                .expect("search row");
            let key = Binding::ctrl_alt(Key::e);
            let _keymap =
                use_keymap(Keymap::defaults().with(Map::Default, SEARCH_SUBFOLDERS, vec![key]));
            direct.enter(Map::Default, row);
            direct.press(key);
            assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Search));
            fixture.shortcuts.type_prompt_text("a.txt");
            assert!(fixture.press(Key::Return, ModifierType::empty()));
            wait_until(|| {
                fixture.view.listing_search_active() && fixture.view.item_view_has_focus()
            });
            wait_until(|| fixture.shortcuts.filter_mark().as_deref() == Some("search: a.txt"));
            assert!(fixture.press(Key::Escape, ModifierType::empty()));
            wait_until(|| !fixture.view.listing_search_active());
            wait_until(|| fixture.shortcuts.filter_mark().is_none());
            wait_until(|| entry_count(&fixture.view.browser()) == direct.names.len());
            assert!(fixture.view.item_view_has_focus());
        },
    );
}

/// While search hits replace the listing, a 10xer pin key pins the hit under
/// the cursor, never the folder the directory cursor hides behind them.
#[test]
fn tenxer_direct_pin_takes_the_search_hit() {
    use super::footer_prompt::{
        ALL_REPORTS, commit_search, cursor_to_hit, enable_tenxer, seed_filter_tree, wait_results,
    };
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::direct_commands::tenxer_direct_pin_takes_the_search_hit",
        || {
            let fixture = KeyboardFixture::new();
            seed_filter_tree(&fixture);
            std::fs::create_dir(fixture._directory.path().join("hidden-folder")).expect("folder");
            fixture.view.refresh();
            let browser = fixture.view.browser();
            wait_until(|| entry_count(&browser) == 8);
            let preferences = enable_tenxer(&fixture);
            preferences.set_type_to_search(false);
            preferences.set_filter_include_subfolders(false);
            let pinned = Rc::new(RefCell::new(Vec::new()));
            let pins = pinned.clone();
            fixture.view.set_pin_handlers(
                Rc::new(move |_location, name, done| {
                    pins.borrow_mut().push(name);
                    done(true);
                }),
                Rc::new(|_location| {}),
                Rc::new(|_| PinStatus::Available),
            );
            let key = Binding::alt(Key::p);
            let _keymap = use_keymap(Keymap::defaults().with(Map::Tenxer, PIN, vec![key]));
            move_to_named(&fixture, &browser, "hidden-folder");
            commit_search(&fixture, "report");
            wait_results(&fixture, &ALL_REPORTS);
            for (hit, expected) in [("alpha-report.txt", &[][..]), ("reports", &["reports"][..])] {
                cursor_to_hit(&fixture, hit);
                assert_eq!(focused_name(&browser), "hidden-folder");
                assert!(fixture.press(Key::p, ModifierType::ALT_MASK));
                pump(100);
                assert_eq!(*pinned.borrow(), expected, "pin on {hit}");
                pinned.borrow_mut().clear();
            }
        },
    );
}

/// A default-map half-page key moves the selection like Page Down, so the
/// map's verbs never act on an item paged out of view.
#[test]
fn default_map_half_page_moves_the_selection() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::direct_commands::default_map_half_page_moves_the_selection",
        || {
            let direct = Direct::new();
            let browser = direct.fixture.view.browser();
            let key = Binding::ctrl_alt(Key::e);
            for row in DEFAULT_ROWS
                .iter()
                .filter(|row| [HALF_PAGE_UP, HALF_PAGE_DOWN].contains(&row.command))
            {
                let _keymap =
                    use_keymap(Keymap::defaults().with(Map::Default, row.command, vec![key]));
                direct.enter(Map::Default, row);
                direct.press(key);
                wait_until(|| focused_name(&browser) != row.start);
                assert_eq!(fill_names(&browser), [focused_name(&browser)], "{row:?}");
            }
        },
    );
}
