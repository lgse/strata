// SPDX-License-Identifier: MIT

use std::collections::HashSet;

use gtk::gdk::{Key, ModifierType as Modifiers};

use super::{
    super::{
        Binding, CUSTOM_ACTIONS, KeyPress, Shown,
        bindings::{self, Entry, Listed, Maps},
        custom_action_slot,
    },
    Command, Group, Kind, Map, registry,
};

const MAPS: [Map; 2] = [Map::Default, Map::Tenxer];

const MANY_ROLES: &str =
    "it closes a pane, leaves the sidebar, clears a filter and edits text; kept as a fixed binding";
const GENERIC: &str = "a key many surfaces read for their own job, kept as a fixed binding";
const GTK_OWNED: &str = "GTK acts on it once the dispatcher yields";
const EXTENDS_SELECTION: &str = "Shift with a motion key extends the selection and stays fixed";

/// Catalog bindings that are not commands, and why.
const NOT_COMMANDS: &[(&str, &str)] = &[
    ("BACKSPACE", MANY_ROLES),
    ("SPACE", GENERIC),
    ("HOME_KEY", GENERIC),
    ("END_KEY", GENERIC),
    ("PAGE_UP_KEY", GENERIC),
    ("PAGE_DOWN_KEY", GENERIC),
    ("EXTEND_UP", EXTENDS_SELECTION),
    ("EXTEND_DOWN", EXTENDS_SELECTION),
    ("EXTEND_PAGE_UP", EXTENDS_SELECTION),
    ("EXTEND_PAGE_DOWN", EXTENDS_SELECTION),
    ("EXTEND_LEFT", EXTENDS_SELECTION),
    ("EXTEND_RIGHT", EXTENDS_SELECTION),
    ("EXTEND_TO_FIRST", EXTENDS_SELECTION),
    ("EXTEND_TO_LAST", EXTENDS_SELECTION),
    ("SELECTION_TOGGLE", GTK_OWNED),
    ("UNSELECT_ALL", GTK_OWNED),
    ("DOCUMENT_START", GTK_OWNED),
    ("DOCUMENT_END", GTK_OWNED),
];

const TIMELINE: &str = "the media timeline reads it in either map";

/// Listed commands bound in both maps although the reference shows them in
/// one, and why.
const LIVE_IN_BOTH_MAPS: &[(&str, &str)] = &[
    ("COMPLETE", "location bar path completion"),
    ("CHOOSE_FOLDER", "folder requests in the file chooser"),
    (
        "TERMINAL",
        "the listing and sidebar open a terminal; only text fields leave it unbound in 10xer",
    ),
    ("MEDIA_SEEK_BACK_PAGE", TIMELINE),
    ("MEDIA_SEEK_FORWARD_PAGE", TIMELINE),
    ("MEDIA_SEEK_START", TIMELINE),
    ("MEDIA_VOLUME_STEP_UP", TIMELINE),
    ("MEDIA_VOLUME_STEP_DOWN", TIMELINE),
    ("MEDIA_SEEK_END", TIMELINE),
    (
        "FIND_IN_PREVIEW",
        "a focused preview opens find with Ctrl+F; the default reference lists no preview keys",
    ),
];

fn bound_maps(command: Command) -> Vec<Map> {
    MAPS.into_iter()
        .filter(|map| !command.defaults(*map).is_empty())
        .collect()
}

fn trigger(group: Group) -> Option<Command> {
    registry().find(|command| command.kind() == Kind::Trigger(group))
}

fn is_trigger_key(prefix: Shown, map: Map) -> bool {
    let Shown::Command(prefix) = prefix else {
        return false;
    };
    matches!(prefix.kind(), Kind::Trigger(_)) && !prefix.defaults(map).is_empty()
}

#[test]
fn listed_commands_are_bound_in_the_maps_that_list_them() {
    let mut checked = 0;
    for constant in bindings::listed() {
        let name = constant.name;
        let block: Vec<Map> = MAPS
            .into_iter()
            .filter(|map| constant.maps.include(*map == Map::Tenxer))
            .collect();
        let live_in_both = LIVE_IN_BOTH_MAPS.iter().any(|(live, _)| *live == name);
        match constant.entry {
            Entry::Command(command) => {
                // Custom action slots ship no key; users bind them.
                let expected = if CUSTOM_ACTIONS.contains(&command) {
                    Vec::new()
                } else if live_in_both {
                    MAPS.to_vec()
                } else {
                    block
                };
                assert_eq!(bound_maps(command), expected, "{name} ({})", command.id());
                assert!(
                    !(live_in_both && constant.maps == Maps::Both),
                    "{name} is listed in both maps, so it needs no LIVE_IN_BOTH_MAPS entry"
                );
                checked += 1;
            }
            // Digit ranges stay fixed; a chord's range belongs to its trigger's group.
            Entry::Digits(digits) => {
                if let Some(prefix) = digits.prefix() {
                    assert!(
                        block.iter().all(|map| is_trigger_key(prefix, *map)),
                        "{name} follows no chord trigger"
                    );
                }
            }
            Entry::Binding(_) => assert!(
                NOT_COMMANDS.iter().any(|(excluded, _)| *excluded == name),
                "{name} is a fixed binding; make it a command or give it a reason"
            ),
        }
    }
    assert!(checked > 0);
    for (name, _) in LIVE_IN_BOTH_MAPS {
        assert!(
            bindings::listed()
                .any(|constant| constant.name == *name
                    && matches!(constant.entry, Entry::Command(_))),
            "{name} is not a listed command"
        );
    }
}

#[test]
fn unlisted_constants_are_commands_or_excluded() {
    let mut checked = 0;
    for (name, entry) in bindings::unlisted() {
        match entry {
            Entry::Command(command) => assert!(
                !bound_maps(command).is_empty(),
                "{} has no binding in either map",
                command.id()
            ),
            Entry::Binding(_) => assert!(
                NOT_COMMANDS.iter().any(|(excluded, _)| *excluded == name),
                "{name} needs to be a command or have a reason on the exclusion list"
            ),
            Entry::Digits(_) => panic!("{name} is an unlisted digit range"),
        }
        checked += 1;
    }
    assert!(checked > 0);
    let fixed: HashSet<&str> = bindings::listed()
        .map(|constant| (constant.name, constant.entry))
        .chain(bindings::unlisted())
        .filter(|(_, entry)| matches!(entry, Entry::Binding(_)))
        .map(|(name, _)| name)
        .collect();
    for (name, reason) in NOT_COMMANDS {
        assert!(
            fixed.contains(name),
            "{name} is excluded as {reason:?} but is not a fixed catalog binding"
        );
    }
}

#[test]
fn ids_are_unique_and_round_trip() {
    fn word(text: &str) -> bool {
        text.starts_with(|character: char| character.is_ascii_lowercase())
            && text.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
            })
    }
    let mut ids = HashSet::new();
    for command in registry() {
        let id = command.id();
        assert!(ids.insert(id), "{id} is used twice");
        assert_eq!(Command::parse(id), Some(command));
        let well_formed = match command.kind() {
            Kind::Single | Kind::Structural => {
                word(id)
                    || custom_action_slot(command)
                        .is_some_and(|number| id == format!("custom_action.{number}"))
            }
            Kind::Trigger(group) => word(id) && id == group.id(),
            Kind::Step(group) => id
                .split_once('.')
                .is_some_and(|(prefix, step)| prefix == group.id() && word(step)),
        };
        assert!(well_formed, "{id} does not fit {:?}", command.kind());
    }
    assert!(!ids.is_empty());
    for unknown in ["", "Copy", "go.", ".home", "go.home.", "future_command"] {
        assert_eq!(Command::parse(unknown), None, "{unknown:?}");
    }
}

#[test]
fn chord_steps_name_their_trigger() {
    let mut steps = 0;
    for command in registry() {
        match command.kind() {
            Kind::Step(group) => {
                let Some(trigger) = trigger(group) else {
                    panic!("{} has no {group:?} trigger", command.id());
                };
                for map in bound_maps(command) {
                    assert!(
                        !trigger.defaults(map).is_empty(),
                        "{} is bound in the {map:?} map, where {} is not",
                        command.id(),
                        trigger.id()
                    );
                }
                steps += 1;
            }
            Kind::Trigger(group) => assert_eq!(
                registry()
                    .filter(|other| other.kind() == Kind::Trigger(group))
                    .count(),
                1,
                "{group:?} has more than one trigger"
            ),
            Kind::Single | Kind::Structural => {}
        }
    }
    assert!(steps > 0);
    let listed: Vec<Listed> = bindings::listed().collect();
    for constant in &listed {
        if let Some(after) = constant.after {
            assert!(
                matches!(after.kind(), Kind::Single),
                "{} follows a chord trigger, so it must be a step",
                constant.name
            );
        }
    }
}

/// A command matches the active map's bindings, which keeps both tables apart
/// even where a key means different things in each.
#[test]
fn commands_match_through_the_active_map() {
    crate::test_support::gtk_test(
        "ui::keys::command::tests::commands_match_through_the_active_map",
        || {
            use super::super::bindings::{
                DUPLICATE, GO_PROMPT, HALF_PAGE_DOWN, MOVE_TO_TRASH, OPEN_IN_NEW_TAB, RENAME,
                SIDEBAR,
            };
            let press = |key, modifiers| KeyPress::from_signal(key, 0, modifiers);
            let ctrl = Modifiers::CONTROL_MASK;
            let preferences = crate::ui::preferences::PreferenceManager::shared();
            for tenxer in [false, true] {
                preferences.set_tenxer_mode(tenxer);
                assert_eq!(
                    Map::active(),
                    if tenxer { Map::Tenxer } else { Map::Default }
                );
                let ctrl_b = press(Key::b, ctrl);
                assert_eq!(ctrl_b.is(SIDEBAR), !tenxer);
                assert!(ctrl_b.is(SIDEBAR.in_map(Map::Default)));
                assert_eq!(press(Key::n, ctrl).is(SIDEBAR), tenxer);
                assert_eq!(press(Key::d, ctrl).is(HALF_PAGE_DOWN), tenxer);
                assert_eq!(press(Key::d, ctrl).is(DUPLICATE), !tenxer);
                assert!(press(Key::d, ctrl).is(DUPLICATE.in_map(Map::Default)));
                assert!(press(Key::F2, Modifiers::empty()).is(RENAME));
                assert_eq!(press(Key::r, Modifiers::empty()).is(RENAME), tenxer);
                assert_eq!(press(Key::r, ctrl).is(RENAME), !tenxer);
                assert!(press(Key::r, ctrl).is(RENAME.in_map(Map::Default)));
                let shift_space = press(Key::space, Modifiers::SHIFT_MASK);
                assert_eq!(
                    shift_space.is(GO_PROMPT.ignoring(Modifiers::SHIFT_MASK)),
                    tenxer
                );
                assert!(!shift_space.is(GO_PROMPT));
                let ctrl_shift_enter = press(Key::Return, ctrl | Modifiers::SHIFT_MASK);
                assert!(!ctrl_shift_enter.is(OPEN_IN_NEW_TAB));
                assert!(ctrl_shift_enter.is(OPEN_IN_NEW_TAB.ignoring(Modifiers::SHIFT_MASK)));
                let trash = MOVE_TO_TRASH
                    .in_map(Map::Default)
                    .ignoring(Modifiers::SHIFT_MASK);
                assert!(press(Key::Delete, Modifiers::SHIFT_MASK).is(trash));
                assert!(!press(Key::d, Modifiers::empty()).is(trash));
            }
            preferences.set_tenxer_mode(false);
        },
    );
}

#[test]
fn no_command_repeats_a_binding() {
    let mut checked = 0;
    for command in registry() {
        for map in MAPS {
            let bindings = command.defaults(map);
            for (index, binding) in bindings.iter().enumerate() {
                assert!(
                    !bindings[..index].contains(binding),
                    "{} lists {binding:?} twice in the {map:?} map",
                    command.id()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 0);
}

#[test]
fn saved_accelerators_read_back_as_the_bindings_they_name() {
    crate::test_support::gtk_test(
        "ui::keys::command::tests::saved_accelerators_read_back_as_the_bindings_they_name",
        || {
            let mut checked = 0;
            for command in registry() {
                for map in MAPS {
                    for binding in command.defaults(map) {
                        assert_eq!(
                            Binding::parse(&binding.accelerator()),
                            Some(*binding),
                            "{} {}",
                            command.id(),
                            binding.accelerator()
                        );
                        checked += 1;
                    }
                }
            }
            assert!(checked > 0);
            let shift = Modifiers::SHIFT_MASK;
            let ctrl = Modifiers::CONTROL_MASK;
            for (accelerator, expected) in [
                ("<Control>e", Some(Binding::ctrl(Key::e))),
                ("<Primary>e", Some(Binding::ctrl(Key::e))),
                ("<Shift>g", Some(Binding::shift(Key::g))),
                ("G", Some(Binding::plain(Key::g))),
                ("<Control>1", Some(Binding::ctrl(Key::_1))),
                ("<Control>KP_1", Some(Binding::ctrl(Key::_1))),
                ("KP_Enter", Some(Binding::plain(Key::Return))),
                ("0", Some(Binding::plain(Key::_0).ignoring(shift))),
                ("<Shift>ISO_Left_Tab", Some(Binding::shift(Key::Tab))),
                (
                    "<Control><Shift>ISO_Left_Tab",
                    Some(Binding::new(Key::Tab, ctrl | shift)),
                ),
                (
                    "<Super>space",
                    Some(Binding::new(Key::space, Modifiers::SUPER_MASK)),
                ),
                ("<Bogus>c", None),
                ("<Meta>c", None),
                ("<Hyper>c", None),
                ("Control_L", None),
                ("", None),
            ] {
                assert_eq!(Binding::parse(accelerator), expected, "{accelerator:?}");
            }
        },
    );
}

#[test]
fn saved_entries_replace_whole_entries_and_skip_what_cannot_apply() {
    crate::test_support::gtk_test(
        "ui::keys::command::tests::saved_entries_replace_whole_entries_and_skip_what_cannot_apply",
        || {
            use super::super::bindings::{COPY, DUPLICATE, ESCAPE, RENAME};
            use super::{Keymap, Rejected};
            let list = |accelerators: &[&str]| {
                toml::Value::Array(
                    accelerators
                        .iter()
                        .map(|accelerator| toml::Value::String((*accelerator).into()))
                        .collect(),
                )
            };
            let entries = [
                ("rename", list(&["<Control>e", "F2", "<Control>E"])),
                ("duplicate", list(&[])),
                ("escape", list(&[])),
                ("copy", list(&["<Bogus>c"])),
                ("future_cmd", list(&["x"])),
                ("cut", toml::Value::String("<Control>x".into())),
                (
                    "go",
                    toml::Value::Table([("home".into(), list(&["x"]))].into_iter().collect()),
                ),
            ];
            let mut rejected = Vec::new();
            let keymap = Keymap::defaults().overridden(
                Map::Default,
                entries.iter().map(|(id, value)| (*id, value)),
                |entry| rejected.push(format!("{entry:?}")),
            );
            assert_eq!(
                keymap.bindings(Map::Default, RENAME),
                [Binding::ctrl(Key::e), Binding::plain(Key::F2)]
            );
            assert!(keymap.bindings(Map::Default, DUPLICATE).is_empty());
            assert!(keymap.bindings(Map::Default, ESCAPE).is_empty());
            assert_eq!(
                keymap.bindings(Map::Default, COPY),
                COPY.defaults(Map::Default)
            );
            assert_eq!(
                keymap.bindings(Map::Tenxer, RENAME),
                RENAME.defaults(Map::Tenxer),
                "an override applies to its own map only"
            );
            assert!(keymap.overrides(Map::Default, RENAME));
            assert!(!keymap.overrides(Map::Default, COPY));
            let bogus = toml::Value::String("<Bogus>c".into());
            assert_eq!(
                rejected,
                [
                    format!("{:?}", Rejected::Accelerator("copy", &bogus)),
                    format!("{:?}", Rejected::UnknownCommand("future_cmd")),
                    format!("{:?}", Rejected::NotAList("cut")),
                    format!("{:?}", Rejected::Table("go")),
                ]
            );
            let _keymap = super::use_keymap(keymap);
            let press = |key, modifiers| KeyPress::from_signal(key, 0, modifiers);
            assert!(press(Key::e, Modifiers::CONTROL_MASK).is(RENAME));
            assert!(!press(Key::r, Modifiers::CONTROL_MASK).is(RENAME));
            assert!(!press(Key::d, Modifiers::CONTROL_MASK).is(DUPLICATE));
            assert!(
                press(Key::Escape, Modifiers::empty()).is(ESCAPE),
                "physical Escape outlives an empty escape entry"
            );
        },
    );
}

/// A matcher that tolerates extra modifiers still needs the ones a binding
/// holds itself, so a refresh rebound to Ctrl+R never fires on plain **r**.
#[test]
fn modifier_tolerant_commands_keep_a_bindings_own_modifiers() {
    use super::super::{SHORTCUT_MODIFIERS, bindings::REFRESH};
    let _keymap = super::use_keymap(super::Keymap::defaults().with(
        Map::Default,
        REFRESH,
        vec![Binding::ctrl(Key::r)],
    ));
    let refresh = REFRESH.ignoring(SHORTCUT_MODIFIERS);
    let press = |key, modifiers| KeyPress::from_signal(key, 0, modifiers);
    let ctrl = Modifiers::CONTROL_MASK;
    for modifiers in [
        Modifiers::empty(),
        Modifiers::SHIFT_MASK,
        Modifiers::ALT_MASK,
    ] {
        assert!(!press(Key::r, modifiers).is(refresh), "{modifiers:?}");
    }
    for modifiers in [
        ctrl,
        ctrl | Modifiers::SHIFT_MASK,
        ctrl | Modifiers::ALT_MASK,
    ] {
        assert!(press(Key::r, modifiers).is(refresh), "{modifiers:?}");
    }
    assert!(!press(Key::F5, Modifiers::empty()).is(refresh));
}

/// Rows that split one command's keys by position show the whole entry on
/// the first alternate once a user changes it, and nothing on the others.
#[test]
fn overridden_alternates_show_every_key_a_user_added() {
    use super::super::{
        Keys,
        bindings::{TAB_NEXT, TAB_PREVIOUS},
        label_in_map, shown,
    };
    let next_row = Keys::Any(shown![TAB_NEXT.alternate(0), TAB_PREVIOUS.alternate(0)]);
    let paging_row = Keys::Any(shown![TAB_PREVIOUS.alternate(1), TAB_NEXT.alternate(1)]);
    let label = |keys: &Keys| label_in_map("en", keys, Map::Default);
    let shipped = (label(&next_row), label(&paging_row));
    let _keymap = super::use_keymap(super::Keymap::defaults().with(
        Map::Default,
        TAB_NEXT,
        vec![
            Binding::ctrl(Key::Tab),
            Binding::ctrl(Key::Page_Down),
            Binding::alt(Key::n),
        ],
    ));
    let (next, paging) = (label(&next_row), label(&paging_row));
    assert!(next.contains("Alt+N"), "{next}");
    assert!(next.contains("Page Down"), "{next}");
    assert_eq!(paging.matches('/').count(), 0, "{paging}");
    assert_ne!((next, paging), shipped);
}
