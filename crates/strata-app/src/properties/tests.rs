// SPDX-License-Identifier: MIT

//! Behavioural tests for the Properties dialog (`super`): command routing across focus, entry,
//! folder background and sidebar places, Escape focus restoration, recursive directory and
//! multi-selection size calculation with symlink skipping and incomplete warnings, permission bit
//! and executable toggles on disk, and footer actions (`Copy path`, `Rename`, `Pin`/`Unpin`).

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use rune::{Key, KeyEvent, Mods, Tx};
use strata_core::model::Location;
use strata_fs::adapters::directory_summary::DirectorySummary;

use super::{
    Dialog, Properties,
    actions::{PinAction, pin_action_for},
    measure::{directory_counts_label, measurement_warning_text, size_label},
    media::media_rows,
    permissions::{toggled_permission, with_execute_permissions},
};
use crate::{
    App,
    commands::{self, Holds, On, Outcome},
    host::offscreen::Offscreen,
    raw_details, shell,
    sidebar::PinState,
    slots::Part,
    test_support::{PATIENCE, fixture, hosted, send_key, view::idle},
};
use strata_preview::sandbox::{
    metadata::{Chapter, MediaMetadata, SubtitleTrack},
    raw_metadata::RawMetadata,
};

const MODULE: &str = "properties::tests";

fn open_fixture(host: &mut Offscreen, root: &Path) {
    host.app().open(Location::local(root));
    let listed = |host: &Offscreen| host.app().browser().column_entry_counts(0).is_some();
    assert!(host.run_until(PATIENCE, listed), "the folder is listed");
    while host.turn() {}
}

fn center_of(host: &mut Offscreen, key: &str) -> (f32, f32) {
    let mounted = |host: &Offscreen| {
        host.elements()
            .iter()
            .any(|el| !el.exiting && el.key.as_deref() == Some(key))
    };
    assert!(host.run_until(PATIENCE, mounted), "{key} is mounted");
    let elements = host.runtime().elements();
    let el = elements
        .iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(key))
        .expect("element");
    let [x, y, w, h] = el.visible;
    (x + w / 2.0, y + h / 2.0)
}

fn click(host: &mut Offscreen, key: &str) {
    let (x, y) = center_of(host, key);
    host.runtime().pointer_move(x, y);
    host.runtime().pointer_down(x, y);
    host.turn();
    host.runtime().pointer_up(x, y);
    idle(host);
}

fn press(host: &mut Offscreen, key: Key, mods: Mods) {
    send_key(host, KeyEvent { key, mods });
    while host.turn() {}
}

fn run_on(host: &mut Offscreen, on: On) -> Outcome {
    let mut out = Outcome::Declined;
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        out = commands::run(app, "properties", on, tx);
    });
    idle(host);
    out
}

fn populate_sized_folder(root: &Path) {
    let sized = root.join("sized-folder");
    fs::create_dir_all(sized.join("nested")).expect("nested");
    fs::create_dir_all(sized.join(".hidden/visible")).expect("hidden visible");
    fs::write(sized.join("top.txt"), b"abc").expect("top.txt");
    fs::write(sized.join("nested/child.txt"), b"12345").expect("child.txt");
    fs::write(sized.join("nested/.hidden.txt"), b"1234567").expect(".hidden.txt");
    fs::write(sized.join(".hidden/visible/file.txt"), b"123").expect("file.txt");
    std::os::unix::fs::symlink(&sized, sized.join("nested/loop")).expect("loop symlink");
    std::os::unix::fs::symlink(root.join("todo.txt"), sized.join("outside"))
        .expect("outside symlink");
}

fn has_a11y_node(node: &serde_json::Value, role: &str, name: &str, state: Option<&str>) -> bool {
    let matches_self = node.get("role").and_then(|v| v.as_str()) == Some(role)
        && node.get("name").and_then(|v| v.as_str()) == Some(name)
        && state.is_none_or(|wanted| {
            node.get("states")
                .and_then(|v| v.as_array())
                .is_some_and(|states| states.iter().any(|s| s.as_str() == Some(wanted)))
        });
    matches_self
        || node
            .get("children")
            .and_then(|v| v.as_array())
            .is_some_and(|children| {
                children
                    .iter()
                    .any(|child| has_a11y_node(child, role, name, state))
            })
}

#[test]
fn summary_and_permission_helpers_format_truncated_counts_warnings_and_bits() {
    let mut summary = DirectorySummary {
        total_size: 18,
        item_count: 7,
        visible_file_count: 4,
        visible_folder_count: 3,
        ..DirectorySummary::default()
    };
    assert_eq!(
        (
            size_label(&summary),
            directory_counts_label(&summary),
            measurement_warning_text(&summary),
        ),
        ("18 B".to_owned(), "4 files, 3 folders".to_owned(), None),
    );

    summary.issues.unreadable = true;
    summary.issues.depth_limited = true;
    let warning = measurement_warning_text(&summary).expect("warning");
    assert_eq!(
        (size_label(&summary), directory_counts_label(&summary)),
        ("≥ 18 B".to_owned(), "≥ 4 files, ≥ 3 folders".to_owned()),
    );
    assert!(
        warning.starts_with("Totals are incomplete.\n")
            && warning.contains("Some folders or entries couldn't be read.")
            && warning.contains("64-level nesting limit")
    );
    assert_eq!(
        (
            toggled_permission(0o644, 0o100),
            with_execute_permissions(0o644, true),
            with_execute_permissions(0o755, false),
        ),
        (0o744, 0o755, 0o644),
    );
}

#[test]
fn pin_action_hides_for_files_trash_and_standard_places() {
    let folder = Location::local(Path::new("/tmp/custom-folder"));
    let trash = Location::uri("trash:///");
    let cases = [
        (&folder, true, PinState::Available, Some(PinAction::Pin)),
        (&folder, true, PinState::Pinned, Some(PinAction::Unpin)),
        (&folder, true, PinState::Unavailable, None),
        (&folder, false, PinState::Available, None),
        (&trash, true, PinState::Available, None),
    ];
    for (location, is_dir, state, expected) in cases {
        assert_eq!(pin_action_for(location, is_dir, state), expected);
    }
}

fn single_state(host: &Offscreen) -> Option<(&str, bool, bool)> {
    let Some(Dialog::Single(single)) = &host.app().properties.dialog else {
        return None;
    };
    Some((&single.name, single.entry.is_some(), single.is_directory))
}

#[test]
fn all_four_command_routes_open_properties_and_escape_restores_focus() {
    hosted(
        MODULE,
        "all_four_command_routes_open_properties_and_escape_restores_focus",
        |mut host| {
            let folder = fixture();
            open_fixture(&mut host, folder.path());

            let selected = host
                .app()
                .browser()
                .select_entries_by_name_at(0, &["todo.txt".to_owned()]);
            assert!(selected);
            while host.turn() {}

            let alt = Mods {
                alt: true,
                ..Mods::default()
            };
            press(&mut host, Key::Enter, alt);
            assert_eq!(
                (
                    host.app().properties.open,
                    <Properties as Part>::keyboard(host.app()),
                    has_a11y_node(&host.tree(), "button", "Close dialog", Some("focused")),
                ),
                (true, Some(Holds::Modal { field: false }), true),
            );

            press(&mut host, Key::Escape, Mods::default());
            assert!(!host.app().properties.open);

            let id = shell::id_of(
                host.app(),
                &Location::local(folder.path().join("readme.md")),
            );
            assert_eq!(
                (run_on(&mut host, On::Row((0, id))), single_state(&host)),
                (Outcome::Ran, Some(("readme.md", true, false))),
            );
            let root_name = Location::local(folder.path()).display_name();
            assert_eq!(
                (run_on(&mut host, On::Column(0)), single_state(&host)),
                (Outcome::Ran, Some((root_name.as_str(), false, true))),
            );

            let place = Location::local(folder.path().join("documents"));
            assert_eq!(
                (run_on(&mut host, On::Place(place)), single_state(&host)),
                (Outcome::Ran, Some(("documents", false, true))),
            );

            press(&mut host, Key::Escape, Mods::default());
            let recent = run_on(&mut host, On::Place(Location::uri("recent:///")));
            assert_eq!((recent, host.app().properties.open), (Outcome::Ran, false));
        },
    );
}

#[test]
fn recursive_folder_and_selection_sizes_skip_symlinks_and_count_hidden() {
    hosted(
        MODULE,
        "recursive_folder_and_selection_sizes_skip_symlinks_and_count_hidden",
        |mut host| {
            let folder = fixture();
            populate_sized_folder(folder.path());
            open_fixture(&mut host, folder.path());

            let sized = folder.path().join("sized-folder");
            let id = shell::id_of(host.app(), &Location::local(&sized));
            assert_eq!(run_on(&mut host, On::Row((0, id))), Outcome::Ran);
            assert!(host.run_until(PATIENCE, |h| {
                matches!(&h.app().properties.dialog, Some(Dialog::Single(s)) if !s.spinning)
            }));
            let Some(Dialog::Single(single)) = &host.app().properties.dialog else {
                panic!("expected single properties");
            };
            assert_eq!(
                (
                    single.size.as_str(),
                    single.contains.as_deref(),
                    single.warning.as_deref()
                ),
                ("18 B", Some("4 files, 1 folder"), None),
            );

            let both = host
                .app()
                .browser()
                .select_entries_by_name_at(0, &["sized-folder".to_owned(), "todo.txt".to_owned()]);
            assert!(both);
            while host.turn() {}
            assert_eq!(run_on(&mut host, On::Focus), Outcome::Ran);
            assert!(host.run_until(PATIENCE, |h| {
                matches!(&h.app().properties.dialog, Some(Dialog::Selection(s)) if !s.spinning)
            }));
            let Some(Dialog::Selection(selection)) = &host.app().properties.dialog else {
                panic!("expected selection properties");
            };
            assert_eq!(
                (
                    selection.title.as_str(),
                    selection.contains.as_str(),
                    selection.warning.as_deref(),
                ),
                ("2 items selected", "5 files, 1 folder", None),
            );
        },
    );
}

#[test]
fn permission_bits_and_executable_checkbox_update_disk_mode() {
    hosted(
        MODULE,
        "permission_bits_and_executable_checkbox_update_disk_mode",
        |mut host| {
            let folder = fixture();
            let target = folder.path().join("todo.txt");
            fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).expect("chmod 644");
            open_fixture(&mut host, folder.path());

            let id = shell::id_of(host.app(), &Location::local(&target));
            assert_eq!(run_on(&mut host, On::Row((0, id))), Outcome::Ran);
            assert!(host.run_until(PATIENCE, |h| {
                matches!(
                    &h.app().properties.dialog,
                    Some(Dialog::Single(s)) if s.permissions.sensitive()
                )
            }));
            let opened = host.app().properties.opened;

            click(&mut host, &format!("properties-{opened}-bit-owner-x"));
            assert!(host.run_until(PATIENCE, |_| {
                fs::metadata(&target).expect("meta").permissions().mode() & 0o100 != 0
            }));

            click(&mut host, &format!("properties-{opened}-executable"));
            assert!(host.run_until(PATIENCE, |_| {
                fs::metadata(&target).expect("meta").permissions().mode() & 0o111 == 0
            }));
        },
    );
}

#[test]
fn copy_path_and_rename_actions_follow_file_and_folder_rules() {
    hosted(
        MODULE,
        "copy_path_and_rename_actions_follow_file_and_folder_rules",
        |mut host| {
            let folder = fixture();
            open_fixture(&mut host, folder.path());

            let doc_path = folder.path().join("documents");
            let doc_id = shell::id_of(host.app(), &Location::local(&doc_path));
            assert_eq!(run_on(&mut host, On::Row((0, doc_id))), Outcome::Ran);
            let opened = host.app().properties.opened;
            click(&mut host, &format!("properties-{opened}-copy-path"));
            assert!(matches!(
                &host.app().properties.dialog,
                Some(Dialog::Single(s)) if s.copied
            ));

            let file_path = folder.path().join("readme.md");
            let file_id = shell::id_of(host.app(), &Location::local(&file_path));
            assert_eq!(run_on(&mut host, On::Row((0, file_id))), Outcome::Ran);
            let opened = host.app().properties.opened;
            click(&mut host, &format!("properties-{opened}-rename"));
            assert!(!host.app().properties.open);
            assert!(host.app().rename.state.has_keyboard());
        },
    );
}

#[test]
fn open_action_on_executable_settles_run_confirmation_immediately() {
    hosted(
        MODULE,
        "open_action_on_executable_settles_run_confirmation_immediately",
        |mut host| {
            let folder = fixture();
            let program = folder.path().join("tool.bin");
            fs::write(&program, b"\x7fELF").expect("write program");
            fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("chmod 755");
            open_fixture(&mut host, folder.path());

            let selected = host
                .app()
                .browser()
                .select_entries_by_name_at(0, &["tool.bin".to_owned()]);
            assert!(selected);
            while host.turn() {}

            let alt = Mods {
                alt: true,
                ..Mods::default()
            };
            press(&mut host, Key::Enter, alt);
            let opened = host.app().properties.opened;
            let (x, y) = center_of(&mut host, &format!("properties-{opened}-open"));
            host.runtime().pointer_move(x, y);
            host.runtime().pointer_down(x, y);
            host.runtime().frame(1.0 / 60.0);
            host.runtime().pointer_up(x, y);
            // Render one frame without pumping `host.turn()` / `poll`, proving `open_clicked`
            // settled `BrowserEvent::OpenRequested` in the button's own commit.
            host.runtime().frame(1.0 / 60.0);
            assert_eq!(
                (
                    host.app().properties.open,
                    has_a11y_node(&host.tree(), "dialog", "Run this program?", None),
                ),
                (false, true),
            );

            press(&mut host, Key::Escape, Mods::default());
            assert!(!has_a11y_node(
                &host.tree(),
                "dialog",
                "Run this program?",
                None,
            ));
        },
    );
}

#[test]
fn media_and_raw_rows_format_streams_tracks_and_camera_fields() {
    let media = MediaMetadata {
        dimensions: Some((320, 180)),
        duration: Some(3661.4),
        bitrate: Some(1_500_000.0),
        video_codec: Some("h264".into()),
        frame_rate: Some(24.0),
        audio_codec: Some("aac".into()),
        sample_rate: Some(48_000.0),
        channels: Some(2),
        subtitle_tracks: vec![SubtitleTrack {
            language: Some("en".into()),
        }],
        chapters: vec![Chapter {
            start: 0.0,
            end: 10.0,
            title: None,
        }],
        ..MediaMetadata::default()
    };
    let expected_media = vec![
        ("RESOLUTION", "320 × 180 pixels".into()),
        ("DURATION", "1:01:01".into()),
        ("BITRATE", "1.50 Mb/s".into()),
        ("VIDEO CODEC", "h264".into()),
        ("FRAME RATE", "24.00 fps".into()),
        ("AUDIO CODEC", "aac".into()),
        ("SAMPLE RATE", "48.0 kHz".into()),
        ("CHANNELS", "2 (Stereo)".into()),
        ("SUBTITLES", "1 (en)".into()),
        ("CHAPTERS", "1".into()),
    ];
    assert_eq!(media_rows(&media), expected_media);

    let raw = RawMetadata {
        dimensions: Some((600, 400)),
        camera: Some("Strata Test Camera".into()),
        lens: Some("Synthetic 50mm lens".into()),
        focal_length: Some(50.0),
        shutter_speed: Some(0.004),
        iso: Some(400.0),
        gps: Some((-12.5, -45.25)),
    };
    let expected_raw = [
        ("DIMENSIONS", "600 × 400 pixels".into()),
        ("CAMERA", "Strata Test Camera".into()),
        ("LENS", "Synthetic 50mm lens".into()),
        ("FOCAL LENGTH", "50 mm".into()),
        ("SHUTTER SPEED", "1/250 s".into()),
        ("ISO", "400".into()),
        ("GPS COORDINATES", "-12.500000, -45.250000".into()),
    ];
    assert_eq!(raw_details::rows(&raw), expected_raw);
    assert!(
        raw_details::default_rows()
            .iter()
            .all(|(_, val)| val == "N/A")
    );
}
