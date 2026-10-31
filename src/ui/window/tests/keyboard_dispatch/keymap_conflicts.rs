// SPDX-License-Identifier: MIT

//! `conflicts.rs` copies the dispatchers' stage order into hand-kept tables.
//! Each row adds one key to one command, drives the real dispatcher in one
//! state, and requires `check` to report an Error exactly where the command
//! does not act cleanly there: it never fires, or it takes the key from the
//! role the shipped keymap gives it.

use super::*;
use crate::services::SandboxedMedia;
use crate::ui::keys::{
    Binding, Command, Keymap, Map,
    bindings::*,
    conflicts::{Policy, Severity, check},
    use_keymap,
};
use crate::ui::preferences::TextSize;
use crate::ui::tenxer_mode::Chord;

/// Serves `.mp4` files as video and everything else as a long document.
struct Previews;

impl PreviewProvider for Previews {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        glib::idle_add_local_once(move || {
            let path = request
                .entry
                .location
                .native_path()
                .expect("local fixture")
                .to_path_buf();
            let (content_type, content) = if path.extension().is_some_and(|ext| ext == "mp4") {
                let media = SandboxedMedia {
                    path,
                    size: request.media_size,
                    backend: crate::sandbox::MediaPreviewBackend::Software,
                    input_owner: None,
                    audio_only: false,
                };
                ("video/mp4", PreviewContent::SandboxedMedia { media })
            } else {
                let content = (0..600).map(|line| format!("line {line}\n")).collect();
                let text = PreviewContent::Text {
                    content,
                    truncated: false,
                };
                ("text/plain", text)
            };
            emit(PreviewEvent::Ready(Preview {
                request_id: request.id,
                entry: request.entry,
                content_type: content_type.into(),
                content,
            }))
        });
        LoadHandle::new(|| {})
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    List,
    /// The list, with a folder to go back to.
    Visited,
    Filter,
    /// The location bar, a text field in either map.
    LocationBar,
    Armed(Chord),
    Document,
    Media,
}

/// What a press changed.
#[derive(Clone, Copy, Debug)]
enum Effect {
    /// The dispatcher let the key on to GTK or an input method.
    Yields,
    Hidden,
    Location,
    Selection,
    Renames,
    Chord,
    Sidebar,
    ViewMode,
    TextSize,
    Sort(SortKey, SortDirection),
    Feedback,
    LocationEdit,
    Find,
    Volume,
    Muted,
    Reference,
    LeavesPreview,
    WindowAction,
}

#[derive(Clone, Debug, PartialEq)]
struct Seen {
    location: Option<Location>,
    selected: Vec<usize>,
    hidden: bool,
    sort: Option<(SortKey, SortDirection)>,
    renaming: bool,
    chord: Option<Chord>,
    sidebar: bool,
    mode: BrowserMode,
    text_size: TextSize,
    feedback: String,
    location_edit: bool,
    find: bool,
    audio: (f64, bool),
    reference: bool,
    preview_focus: bool,
    window_actions: usize,
}

impl Effect {
    fn seen(self, before: &Seen, after: &Seen, handled: bool) -> bool {
        match self {
            Self::Yields => !handled,
            Self::Hidden => before.hidden != after.hidden,
            Self::Location => before.location != after.location,
            Self::Selection => before.selected != after.selected,
            Self::Renames => after.renaming && !before.renaming,
            Self::Chord => after.chord.is_some() && before.chord != after.chord,
            Self::Sidebar => before.sidebar != after.sidebar,
            Self::ViewMode => before.mode != after.mode,
            Self::TextSize => before.text_size != after.text_size,
            Self::Sort(key, direction) => {
                before.sort != after.sort && after.sort == Some((key, direction))
            }
            Self::Feedback => !after.feedback.is_empty() && before.feedback != after.feedback,
            Self::LocationEdit => after.location_edit && !before.location_edit,
            Self::Find => after.find && !before.find,
            Self::Volume => before.audio.0 != after.audio.0,
            Self::Muted => before.audio.1 != after.audio.1,
            Self::Reference => before.reference != after.reference,
            Self::LeavesPreview => before.preview_focus && !after.preview_focus,
            Self::WindowAction => before.window_actions != after.window_actions,
        }
    }
}

#[derive(Debug)]
struct Row {
    map: Map,
    state: State,
    command: Command,
    key: &'static str,
    /// What the command does when it acts.
    acts: Effect,
    /// What the key does in `state` before the command takes it.
    owner: Option<Effect>,
}

/// The press a binding is recorded from.
fn event(binding: Binding) -> (Key, ModifierType) {
    let modifiers = binding.modifiers();
    let key = if modifiers.contains(ModifierType::SHIFT_MASK) {
        binding.key().to_upper()
    } else {
        binding.key()
    };
    (key, modifiers)
}

/// The table with `binding` added to `command`'s entry, and whether `check`
/// refuses that.
fn added(map: Map, command: Command, binding: Binding) -> (Keymap, bool, String) {
    let mut entry = command.defaults(map).to_vec();
    entry.push(binding);
    let keymap = Keymap::defaults().with(map, command, entry);
    let findings = check(&keymap, map, command, binding, Policy::default());
    let error = findings
        .iter()
        .any(|finding| finding.severity == Severity::Error);
    (keymap, error, format!("{findings:?}"))
}

struct Window {
    fixture: KeyboardFixture,
    preferences: Rc<PreferenceManager>,
    root: Location,
    visited: Location,
    window_actions: Rc<Cell<usize>>,
}

impl Window {
    fn new() -> Self {
        crate::ui::media::use_test_streams(true);
        let fixture = KeyboardFixture::with_provider(Rc::new(Previews));
        let root = fixture._directory.path().to_path_buf();
        std::fs::create_dir(root.join("d")).expect("folder");
        // GIO types an empty file as zero-size; a real MP4 signature keeps it a video.
        std::fs::write(
            root.join("e.mp4"),
            b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isom",
        )
        .expect("clip");
        let preferences = super::footer_prompt::enable_tenxer(&fixture);
        preferences.set_type_to_search(false);
        preferences.set_group_by_type(false);
        preferences.set_preview_autoplay(false);
        let browser = fixture.view.browser();
        fixture.preview.observe_browser(&browser);
        fixture.view.refresh();
        wait_loaded(&browser, 0);
        browser.set_folders_first(0, false);
        let window_actions = Rc::new(Cell::new(0));
        for name in ["jump-folder", "toggle-arrow-scope"] {
            let action = gio::SimpleAction::new(name, None);
            let runs = window_actions.clone();
            action.connect_activate(move |_, _| runs.set(runs.get() + 1));
            fixture.window.add_action(&action);
        }
        Self {
            fixture,
            preferences,
            root: Location::local(&root),
            visited: Location::local(root.join("d")),
            window_actions,
        }
    }

    fn seen(&self) -> Seen {
        let fixture = &self.fixture;
        let browser = fixture.view.browser();
        let column = browser.column_preferences(0);
        Seen {
            location: browser.active_location(),
            selected: fixture.selected(),
            hidden: column.as_ref().is_some_and(|column| column.show_hidden),
            sort: column.map(|column| column.sort()),
            renaming: fixture.view.rename_is_active(),
            chord: fixture.shortcuts.armed_chord(),
            sidebar: fixture.sidebar_toggle.is_active(),
            mode: fixture.view.view_mode(),
            text_size: self.preferences.text_size(),
            feedback: fixture.shortcuts.feedback_text(),
            location_edit: fixture.view.location_has_focus(),
            find: widget_with_class(&fixture.preview.widget(), "preview-find")
                .is_some_and(|find| find.is_visible()),
            audio: (
                self.preferences.preview_volume(),
                self.preferences.preview_muted(),
            ),
            reference: fixture.shortcuts.reference_is_open(),
            preview_focus: preview_has_focus(fixture),
            window_actions: self.window_actions.get(),
        }
    }

    fn navigate(&self, location: &Location) {
        let browser = self.fixture.view.browser();
        if browser.active_location().as_ref() != Some(location) {
            browser.navigate(location.clone());
            wait_until(|| browser.active_location().as_ref() == Some(location));
            wait_loaded(&browser, 0);
        }
    }

    fn enter(&self, map: Map, state: State) {
        let fixture = &self.fixture;
        let preferences = &self.preferences;
        let browser = fixture.view.browser();
        if fixture.shortcuts.reference_is_open() {
            fixture.press(Key::F1, ModifierType::empty());
        }
        fixture.shortcuts.cancel_chord();
        fixture.shortcuts.dismiss_prompt();
        fixture.view.cancel_rename();
        if fixture.view.location_has_focus() {
            fixture.view.cancel_location_edit();
        }
        if fixture.view.show_filter() {
            wait_until(|| fixture.view.filter_has_focus());
            fixture.view.dismiss_focused_filter();
        }
        fixture.preview.close();
        preferences.set_tenxer_mode(map == Map::Tenxer);
        preferences.set_text_size(TextSize::default());
        preferences.set_preview_audio(0.5, false);
        if browser
            .column_preferences(0)
            .is_some_and(|column| column.show_hidden)
        {
            browser.toggle_hidden();
        }
        fixture.sidebar_toggle.set_active(true);
        fixture.view.set_view_mode(BrowserMode::List);
        if state == State::Visited {
            self.navigate(&self.visited);
        }
        self.navigate(&self.root);
        browser.set_sort(0, SortKey::Name, SortDirection::Ascending);
        fixture.shortcuts.dismiss_feedback();
        wait_until(|| {
            browser
                .column_snapshot(0)
                .is_some_and(|column| !column.loading && column.count == 5)
                && browser.column_preferences(0).is_some_and(|column| {
                    column.sort() == (SortKey::Name, SortDirection::Ascending)
                })
                && !fixture.view.rename_is_active()
                && rendered_name(&fixture.view.widget(), "a.txt")
        });
        match state {
            State::List | State::Visited => select_named(fixture, "a.txt"),
            State::Filter => {
                focus_files(fixture);
                assert!(fixture.view.show_filter());
                wait_until(|| fixture.view.filter_has_focus());
            }
            State::LocationBar => {
                focus_files(fixture);
                fixture.view.begin_location_edit();
                wait_until(|| fixture.view.location_has_focus());
            }
            State::Armed(chord) => {
                select_named(fixture, "a.txt");
                fixture.shortcuts.arm_chord(chord);
            }
            State::Document => {
                select_named(fixture, "a.txt");
                assert!(fixture.press(Key::space, ModifierType::empty()));
                let document = || {
                    widget_with_class(&fixture.preview.widget(), "preview-virtual-list")
                        .filter(gtk::Widget::is_mapped)
                };
                wait_until(|| document().is_some());
                assert!(document().expect("document").grab_focus());
            }
            State::Media => {
                select_named(fixture, "e.mp4");
                fixture.press(Key::l, ModifierType::empty());
                wait_until(|| {
                    preview_has_focus(fixture)
                        && widget_with_class(&fixture.preview.widget(), "preview-video").is_some()
                });
            }
        }
    }

    /// Presses `binding` in `state` and waits a moment for any of `effects`.
    fn press(
        &self,
        map: Map,
        state: State,
        binding: Binding,
        effects: &[Effect],
    ) -> (Seen, Seen, bool) {
        self.enter(map, state);
        let before = self.seen();
        let (key, modifiers) = event(binding);
        let handled = self.fixture.press(key, modifiers);
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut after = self.seen();
        while !effects
            .iter()
            .any(|effect| effect.seen(&before, &after, handled))
            && Instant::now() < deadline
        {
            pump(10);
            after = self.seen();
        }
        (before, after, handled)
    }

    fn agrees(&self, row: &Row) {
        let binding = Binding::parse(row.key).expect("accelerator");
        if let Some(shipped) = row.command.defaults(row.map).first() {
            let (before, after, handled) = self.press(row.map, row.state, *shipped, &[row.acts]);
            assert!(
                row.acts.seen(&before, &after, handled),
                "{row:?}: the shipped key shows no {:?}: {before:?} -> {after:?}",
                row.acts
            );
        }
        let (keymap, error, findings) = added(row.map, row.command, binding);
        let effects: Vec<Effect> = row.owner.into_iter().chain([row.acts]).collect();
        let (before, after, handled) = {
            let _keymap = use_keymap(keymap);
            self.press(row.map, row.state, binding, &effects)
        };
        let fired = row.acts.seen(&before, &after, handled);
        if let Some(owner) = row.owner
            && !owner.seen(&before, &after, handled)
        {
            let (before, after, handled) = self.press(row.map, row.state, binding, &[owner]);
            assert!(
                owner.seen(&before, &after, handled),
                "{row:?}: the key's owner shows no {owner:?}: {before:?} -> {after:?}"
            );
        }
        assert_eq!(
            error,
            !fired || row.owner.is_some(),
            "{row:?}: fired={fired}, findings={findings}, {before:?} -> {after:?}"
        );
    }
}

const ROWS: &[Row] = &[
    // Duplicate: the same window surface reads rename before hidden files.
    Row {
        map: Map::Default,
        state: State::List,
        command: RENAME,
        key: "<Control>h",
        acts: Effect::Renames,
        owner: Some(Effect::Hidden),
    },
    // FixedKey(Backspace)
    Row {
        map: Map::Default,
        state: State::List,
        command: RENAME,
        key: "BackSpace",
        acts: Effect::Renames,
        owner: Some(Effect::Location),
    },
    // ReservedByGtk
    Row {
        map: Map::Default,
        state: State::List,
        command: RENAME,
        key: "<Shift>Down",
        acts: Effect::Renames,
        owner: Some(Effect::Yields),
    },
    // The directory stage reads history before it refuses Ctrl.
    Row {
        map: Map::Default,
        state: State::Visited,
        command: HISTORY_BACK,
        key: "<Control>e",
        acts: Effect::Location,
        owner: None,
    },
    Row {
        map: Map::Tenxer,
        state: State::List,
        command: HOME_FOLDER,
        key: "<Control>e",
        acts: Effect::Location,
        owner: None,
    },
    // The direct stage runs a step with no chord in the default map.
    Row {
        map: Map::Default,
        state: State::List,
        command: SORT_BY_SIZE,
        key: "<Control><Alt>e",
        acts: Effect::Sort(SortKey::Size, SortDirection::Ascending),
        owner: None,
    },
    Row {
        map: Map::Default,
        state: State::List,
        command: INVERT_SELECTION,
        key: "<Control><Alt>e",
        acts: Effect::Selection,
        owner: None,
    },
    // It reads keys before inline editing.
    Row {
        map: Map::Default,
        state: State::List,
        command: RESTORE,
        key: "F2",
        acts: Effect::Feedback,
        owner: Some(Effect::Renames),
    },
    // So do custom action slots, which say when theirs is empty.
    Row {
        map: Map::Default,
        state: State::List,
        command: CUSTOM_ACTION_1,
        key: "F2",
        acts: Effect::Feedback,
        owner: Some(Effect::Renames),
    },
    // NoPath: 10xer runs custom actions from its Action chord only.
    Row {
        map: Map::Tenxer,
        state: State::List,
        command: CUSTOM_ACTION_1,
        key: "<Control><Alt>e",
        acts: Effect::Feedback,
        owner: None,
    },
    // NoPath: the default map runs no Go step.
    Row {
        map: Map::Default,
        state: State::List,
        command: GO_HOME,
        key: "<Control><Alt>e",
        acts: Effect::Location,
        owner: None,
    },
    // The focused 10xer listing passes a direct command's typed key on.
    Row {
        map: Map::Tenxer,
        state: State::List,
        command: JUMP_TO_FOLDER,
        key: "w",
        acts: Effect::WindowAction,
        owner: None,
    },
    // A 10xer text field yields to the window action its table binds.
    Row {
        map: Map::Tenxer,
        state: State::LocationBar,
        command: ARROW_SCOPE,
        key: "<Control><Alt>e",
        acts: Effect::WindowAction,
        owner: None,
    },
    // NeverMatches: the default map has no chords.
    Row {
        map: Map::Default,
        state: State::List,
        command: GO,
        key: "<Control>e",
        acts: Effect::Chord,
        owner: None,
    },
    // InputMethodKey: location acts in text fields.
    Row {
        map: Map::Default,
        state: State::Filter,
        command: LOCATION,
        key: "<Super>space",
        acts: Effect::LocationEdit,
        owner: Some(Effect::Yields),
    },
    // Text size runs before every surface.
    Row {
        map: Map::Default,
        state: State::Document,
        command: FIND_IN_PREVIEW,
        key: "<Control>equal",
        acts: Effect::Find,
        owner: Some(Effect::TextSize),
    },
    // ShadowsChordGroup
    Row {
        map: Map::Tenxer,
        state: State::List,
        command: SIDEBAR,
        key: "g",
        acts: Effect::Sidebar,
        owner: Some(Effect::Chord),
    },
    // FixedKey(SelectionToggle)
    Row {
        map: Map::Tenxer,
        state: State::List,
        command: SIDEBAR,
        key: "space",
        acts: Effect::Sidebar,
        owner: Some(Effect::Selection),
    },
    // TENXER_PASSES: the listing keeps typed keys from later stages.
    Row {
        map: Map::Tenxer,
        state: State::List,
        command: SIDEBAR,
        key: "e",
        acts: Effect::Sidebar,
        owner: None,
    },
    // TENXER_PASSES: a modified key reaches the later stage.
    Row {
        map: Map::Tenxer,
        state: State::List,
        command: VIEW_COLUMNS,
        key: "<Control>e",
        acts: Effect::ViewMode,
        owner: None,
    },
    // An armed chord reads text size before its steps.
    Row {
        map: Map::Tenxer,
        state: State::Armed(Chord::Go),
        command: GO_HOME,
        key: "<Control>equal",
        acts: Effect::Location,
        owner: Some(Effect::TextSize),
    },
    // FixedKey(Digits): steps are read before pins.
    Row {
        map: Map::Tenxer,
        state: State::Armed(Chord::Go),
        command: GO_HOME,
        key: "1",
        acts: Effect::Location,
        owner: Some(Effect::Feedback),
    },
    // ShiftedSortStep
    Row {
        map: Map::Tenxer,
        state: State::Armed(Chord::Sort),
        command: SORT_BY_SIZE,
        key: "<Shift>a",
        acts: Effect::Sort(SortKey::Size, SortDirection::Ascending),
        owner: Some(Effect::Sort(SortKey::Name, SortDirection::Descending)),
    },
    // FixedKey(MediaKeys): Home seeks before volume steps.
    Row {
        map: Map::Tenxer,
        state: State::Media,
        command: MEDIA_VOLUME_STEP_UP,
        key: "Home",
        acts: Effect::Volume,
        owner: None,
    },
    // The media preview hands keys back before its media commands.
    Row {
        map: Map::Tenxer,
        state: State::Media,
        command: TENXER_MEDIA_MUTE,
        key: "<Shift>Tab",
        acts: Effect::Muted,
        owner: Some(Effect::LeavesPreview),
    },
    // The reference toggles before any preview's media keys.
    Row {
        map: Map::Tenxer,
        state: State::Media,
        command: TENXER_MEDIA_MUTE,
        key: "F1",
        acts: Effect::Muted,
        owner: Some(Effect::Reference),
    },
];

#[test]
fn window_stage_order_matches_the_conflict_check() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_conflicts::window_stage_order_matches_the_conflict_check",
        || {
            let window = Window::new();
            for row in ROWS {
                window.agrees(row);
            }
            crate::ui::media::use_test_streams(false);
        },
    );
}

/// A tabbed window's strip takes its keys, digits included, before any pane.
#[test]
fn tab_strip_order_matches_the_conflict_check() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::keymap_conflicts::tab_strip_order_matches_the_conflict_check",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_tenxer_mode(false);
            let application =
                gtk::Application::new(None::<&str>, gtk::gio::ApplicationFlags::NON_UNIQUE);
            application
                .register(None::<&gtk::gio::Cancellable>)
                .expect("test application");
            let window = gtk::ApplicationWindow::builder()
                .application(&application)
                .default_width(1000)
                .default_height(700)
                .build();
            let tabs =
                super::super::super::composition::TabWindow::new(&window, &preferences, false);
            window.present();
            let root = tempfile::tempdir().expect("fixture");
            std::fs::write(root.path().join("a.txt"), b"a").expect("fixture file");
            let press = |(key, modifiers)| {
                press_phase(&window, gtk::PropagationPhase::Capture, key, modifiers)
            };
            let show_root = || {
                let pane = tabs.active_browser();
                let browser = pane.browser();
                browser.navigate(Location::local(root.path()));
                wait_until(|| {
                    browser.active_location() == Some(Location::local(root.path()))
                        && browser
                            .column_snapshot(0)
                            .is_some_and(|column| !column.loading && column.count == 1)
                });
                browser.select(0, 0);
                browser.focus_active();
                wait_until(|| pane.item_view_has_focus() && rendered_name(&pane.widget(), "a.txt"));
                pane
            };
            show_root();
            assert!(press((Key::t, ModifierType::CONTROL_MASK)));
            let home = Some(Location::local(home_directory()));
            // The second tab is active, so the first tab's digit switches.
            for (map, command, key) in [
                (Map::Default, RENAME, "<Control><Shift>1"),
                (Map::Tenxer, GO_HOME, "<Control>t"),
            ] {
                let binding = Binding::parse(key).expect("accelerator");
                let enter = || {
                    tabs.active_browser().cancel_rename();
                    preferences.set_tenxer_mode(map == Map::Tenxer);
                    let pane = show_root();
                    if command == GO_HOME {
                        assert!(press((Key::g, ModifierType::empty())));
                    }
                    pane
                };
                let acts = |pane: &BrowserView| {
                    pane.rename_is_active() || pane.browser().active_location() == home
                };
                let pane = enter();
                assert!(press(event(command.defaults(map)[0])));
                wait_until(|| acts(&pane));
                let pane = enter();
                let switched = || tabs.active_browser().widget() != pane.widget();
                let settle = || {
                    let deadline = Instant::now() + Duration::from_millis(300);
                    while !switched() && !acts(&pane) && Instant::now() < deadline {
                        pump(10);
                    }
                };
                let (keymap, error, findings) = added(map, command, binding);
                {
                    let _keymap = use_keymap(keymap);
                    press(event(binding));
                    settle();
                }
                let fired = acts(&pane);
                if !switched() {
                    let pane = enter();
                    press(event(binding));
                    settle();
                    assert!(
                        tabs.active_browser().widget() != pane.widget(),
                        "{key}: the tab strip kept no key"
                    );
                }
                assert!(error, "{key}: fired={fired}, findings={findings}");
            }
        },
    );
}
