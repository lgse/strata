// SPDX-License-Identifier: MIT

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use rune::{Key, Mods, Tx};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_platform::csd::{Backend, Csd, Policy};

use super::{
    NAME_KEY, accept,
    geometry::{FALLBACK_CHOOSER_HEIGHT, FALLBACK_CHOOSER_WIDTH, chooser_initial_dimensions},
    options::{ChoiceControl, build_choices, normalize_portal_filters},
    source::{ChooserFileSource, CompiledFilter, chooser_entry_allowed, glob_matches},
};
use crate::{
    App,
    host::offscreen::Offscreen,
    portal::{
        ChooserKind, ChooserRequest, FilterRule, PortalChoice, PortalError, PortalFileFilter,
        SelectedResult, local_uri,
    },
    startup::{self, UiThread},
    test_support::{
        PATIENCE, fixture, in_own_process,
        view::{idle, press},
    },
};

fn file_entry(path: &str, kind: EntryKind) -> FileEntry {
    let pb = PathBuf::from(path);
    let name = pb.file_name().unwrap_or_default().to_os_string();
    FileEntry {
        location: Location::local(&pb),
        thumbnail_path: None,
        native_name: name.clone(),
        display_name: name.to_string_lossy().into_owned(),
        kind,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
        recent_uri: None,
    }
}

#[test]
fn glob_patterns_match_as_the_portal_spells_them() {
    for (pattern, name, matches) in [
        ("*.rs", "main.rs", true),
        ("*.rs", "main.txt", false),
        ("**.rs", "main.rs", true),
        ("*", "", true),
        ("?", "", false),
        ("report-[0-9]?.csv", "report-1a.csv", true),
        ("report-[!0-9].csv", "report-3.csv", false),
        ("report-[^0-9].csv", "report-x.csv", true),
        ("[a-c]x", "bx", true),
        ("[a-c]x", "dx", false),
        ("[x-]", "-", true),
        ("[]]", "]", true),
        ("[abc", "[abc", true),
        ("[abc", "a", false),
        ("[ü]", "ü", true),
    ] {
        assert_eq!(
            glob_matches(pattern, name),
            matches,
            "{pattern:?} on {name:?}"
        );
    }
}

#[test]
fn glob_and_mime_filters_match_files_and_keep_directories() {
    let png_filter = CompiledFilter::new(PortalFileFilter {
        label: "PNG images".into(),
        rules: vec![FilterRule::MimeType("image/png".into())],
    });
    let png = file_entry("/tmp/photo.png", EntryKind::File);
    let txt = file_entry("/tmp/readme.txt", EntryKind::File);
    let dir = file_entry("/tmp/pictures", EntryKind::Directory);

    assert!(chooser_entry_allowed(Some(&png_filter), false, None, &png));
    assert!(!chooser_entry_allowed(Some(&png_filter), false, None, &txt));
    assert!(chooser_entry_allowed(Some(&png_filter), false, None, &dir));
    assert!(!chooser_entry_allowed(None, true, None, &png));
    assert!(chooser_entry_allowed(
        None,
        false,
        Some(PathBuf::from("/media/usb").as_path()),
        &file_entry("/media/usb/photo.png", EntryKind::File),
    ));
    assert!(!chooser_entry_allowed(
        None,
        false,
        Some(PathBuf::from("/media/usb").as_path()),
        &png,
    ));
}

#[test]
fn portal_filters_and_choices_normalize_initial_selections() {
    let text = PortalFileFilter {
        label: "Text".into(),
        rules: vec![FilterRule::Pattern("*.txt".into())],
    };
    let extra = PortalFileFilter {
        label: "Images".into(),
        rules: vec![FilterRule::MimeType("image/*".into())],
    };
    let (compiled, selected) = normalize_portal_filters(&[text], Some(&extra));
    assert_eq!(compiled.len(), 2);
    assert_eq!(selected, Some(1));

    let choices = build_choices(&[
        PortalChoice {
            id: "ro".into(),
            label: "Read only".into(),
            pairs: Vec::new(),
            initial_selection: "true".into(),
        },
        PortalChoice {
            id: "enc".into(),
            label: "Encoding".into(),
            pairs: vec![
                ("utf8".into(), "UTF-8".into()),
                ("latin1".into(), "ISO-8859-1".into()),
            ],
            initial_selection: "latin1".into(),
        },
    ]);
    assert_eq!(
        choices.iter().map(ChoiceControl::value).collect::<Vec<_>>(),
        vec![
            ("ro".to_owned(), "true".to_owned()),
            ("enc".to_owned(), "latin1".to_owned()),
        ]
    );
}

#[test]
fn chooser_dimensions_clamp_to_monitor_and_parent_bounds() {
    assert_eq!(
        chooser_initial_dimensions(None, None),
        (FALLBACK_CHOOSER_WIDTH, FALLBACK_CHOOSER_HEIGHT)
    );
    assert_eq!(
        chooser_initial_dimensions(Some((1920, 1080)), None),
        (1000, 680)
    );
    assert_eq!(
        chooser_initial_dimensions(Some((1920, 1080)), Some((800, 600))),
        (640, 468)
    );
}

const MODULE: &str = "chooser::tests";

fn ctrl() -> Mods {
    Mods {
        ctrl: true,
        ..Mods::default()
    }
}

type Chosen = Rc<RefCell<Option<Result<SelectedResult, PortalError>>>>;

fn run_chooser(
    name: &str,
    settings_extra: &str,
    build_request: impl FnOnce(&std::path::Path) -> (ChooserRequest, Option<PathBuf>),
    body: impl FnOnce(Offscreen, Chosen, tempfile::TempDir),
) {
    in_own_process(&format!("{MODULE}::{name}"), &[], || {
        let config_file = strata_core::storage::config_directory().join("settings.toml");
        std::fs::create_dir_all(config_file.parent().expect("config dir")).expect("dir");
        std::fs::write(
            config_file,
            format!("mode = \"theme\"\ntheme = \"azure-glow\"\nbrowser_mode = \"list\"\n{settings_extra}"),
        )
        .expect("settings");
        let settings = startup::load_settings();
        let ui = UiThread::claim().expect("UI thread");
        let folder = fixture();
        let (request, root_limit) = build_request(folder.path());
        let initial = request.initial_directory.clone();
        let chosen: Chosen = Rc::new(RefCell::new(None));
        let slot = chosen.clone();
        let csd = Csd::new(Policy::Auto.frame(), Backend::Headless);
        let app = App::new_chooser(
            &ui,
            csd,
            settings,
            (
                request,
                ChooserFileSource::new(root_limit),
                None,
                Box::new(move |res| {
                    *slot.borrow_mut() = Some(res);
                }),
            ),
        );
        let mut host = Offscreen::from_app(ui, app, (1200.0, 800.0));
        host.app().open(Location::local(&initial));
        let listed = host.run_until(PATIENCE, |host| {
            host.app().browser().column_entry_counts(0).is_some()
        });
        assert!(listed, "initial chooser listing");
        idle(&mut host);
        body(host, chosen, folder);
    });
}

fn type_text(host: &mut Offscreen, text: &str) {
    for ch in text.chars() {
        press(host, Key::Text(ch.to_string()), Mods::default());
    }
}

fn chooser_request(
    initial_directory: PathBuf,
    kind: ChooserKind,
    filters: Vec<PortalFileFilter>,
) -> ChooserRequest {
    let current_filter = filters.first().cloned();
    ChooserRequest {
        token: "chooser-search-test".into(),
        title: "Choose".into(),
        accept_label: "Accept".into(),
        modal: true,
        parent: None,
        parent_size_hint: None,
        initial_directory,
        kind,
        filters,
        current_filter,
        choices: Vec::new(),
    }
}

fn search_and_wait(host: &mut Offscreen, query: &str) {
    press(host, Key::Text("k".into()), ctrl());
    assert!(
        host.app().palette.state().is_open(),
        "Ctrl+K opens global search"
    );
    type_text(host, query);
    let settled = host.run_until(PATIENCE, |host| {
        let state = host.app().palette.state();
        !state.busy() && !state.items().is_empty()
    });
    assert!(settled, "global search results arrived for {query:?}");
}

fn wait_revealed(host: &mut Offscreen, folder: &std::path::Path, focused: &str) {
    let target = Location::local(folder);
    let revealed = host.run_until(PATIENCE, |host| {
        !host.app().palette.state().is_open()
            && host.app().browser().active_location() == Some(target.clone())
            && host
                .app()
                .browser()
                .focused_entry()
                .is_some_and(|entry| entry.display_name == focused)
    });
    assert!(revealed, "revealed {focused:?} in {}", folder.display());
}

fn accept_uris(host: &mut Offscreen, chosen: &Chosen) -> Vec<String> {
    host.runtime().update(|app: &mut App, tx: &mut Tx| {
        accept::accept(app, tx);
    });
    assert!(
        host.run_until(PATIENCE, |_| chosen.borrow().is_some()),
        "chooser request finishes"
    );
    chosen
        .borrow()
        .as_ref()
        .expect("result")
        .as_ref()
        .expect("ok")
        .uris
        .clone()
}

#[test]
fn global_search_enters_a_folder_without_submitting_the_save_name() {
    run_chooser(
        "global_search_enters_a_folder_without_submitting_the_save_name",
        "",
        |dir| {
            let kind = ChooserKind::SaveFile {
                current_name: Some("report.md".into()),
            };
            (chooser_request(dir.to_path_buf(), kind, Vec::new()), None)
        },
        |mut host, chosen, _folder| {
            let destination = crate::launch::home_directory().join("report-archive");
            std::fs::create_dir_all(&destination).expect("search destination");

            host.runtime().update(|app: &mut App, tx: &mut Tx| {
                let active = app.chooser.active.as_mut().expect("active chooser");
                active.name.replace_text("new-report.md");
                active.name.focused = true;
                tx.focus(NAME_KEY, true);
            });
            idle(&mut host);

            search_and_wait(&mut host, "report-archive");
            press(&mut host, Key::Enter, Mods::default());
            let navigated = host.run_until(PATIENCE, |host| {
                !host.app().palette.state().is_open()
                    && host.app().browser().active_location() == Some(Location::local(&destination))
            });
            assert!(navigated, "Enter navigates into the folder hit");
            assert!(chosen.borrow().is_none(), "search must not submit");
            assert_eq!(
                host.app()
                    .chooser
                    .active
                    .as_ref()
                    .expect("active")
                    .name
                    .text,
                "new-report.md",
                "entering a folder keeps the edited Save name"
            );

            let expected = local_uri(&destination.join("new-report.md")).expect("uri");
            assert_eq!(accept_uris(&mut host, &chosen), vec![expected]);
        },
    );
}

#[test]
fn global_search_reveals_filtered_file_without_launching_or_submitting() {
    run_chooser(
        "global_search_reveals_filtered_file_without_launching_or_submitting",
        "search_open_files_directly = true\n",
        |dir| {
            let filter = PortalFileFilter {
                label: "Text".into(),
                rules: vec![FilterRule::Pattern("*.txt".into())],
            };
            let kind = ChooserKind::Open {
                directory: false,
                multiple: false,
            };
            (chooser_request(dir.to_path_buf(), kind, vec![filter]), None)
        },
        |mut host, chosen, _folder| {
            let home = crate::launch::home_directory();
            let reports = home.join("reports");
            std::fs::create_dir_all(&reports).expect("reports dir");
            let txt = reports.join("quarterly.txt");
            std::fs::write(&txt, "summary").expect("txt");
            std::fs::write(reports.join("quarterly.png"), "excluded").expect("png");

            host.app_mut()
                .chooser
                .active
                .as_mut()
                .expect("active")
                .filter_open = true;
            press(&mut host, Key::Text("k".into()), ctrl());
            assert!(
                !host.app().palette.state().is_open(),
                "open dropdown blocks Ctrl+K"
            );
            host.app_mut()
                .chooser
                .active
                .as_mut()
                .expect("active")
                .filter_open = false;

            press(&mut host, Key::Text("k".into()), ctrl());
            assert!(host.app().palette.state().is_open(), "Ctrl+K opens palette");
            press(&mut host, Key::Text("k".into()), ctrl());
            assert!(
                !host.app().palette.state().is_open(),
                "Ctrl+K closes palette"
            );
            press(&mut host, Key::Text("k".into()), ctrl());
            press(&mut host, Key::Escape, Mods::default());
            assert!(
                !host.app().palette.state().is_open(),
                "Escape closes only palette"
            );
            assert!(chosen.borrow().is_none(), "Escape keeps chooser open");

            search_and_wait(&mut host, "reports");
            press(
                &mut host,
                Key::Enter,
                Mods {
                    alt: true,
                    ..Mods::default()
                },
            );
            wait_revealed(&mut host, &home, "reports");

            search_and_wait(&mut host, "quarterly");
            let labels: Vec<&str> = host
                .app()
                .palette
                .state()
                .items()
                .iter()
                .map(|item| item.label.as_str())
                .collect();
            assert!(
                labels.contains(&"quarterly.txt"),
                "matching .txt shown: {labels:?}"
            );
            assert!(
                !labels.contains(&"quarterly.png"),
                "non-matching .png excluded: {labels:?}"
            );

            press(&mut host, Key::Enter, Mods::default());
            wait_revealed(&mut host, &reports, "quarterly.txt");
            assert!(
                chosen.borrow().is_none(),
                "reveal must not submit the request"
            );

            let expected = local_uri(&txt).expect("uri");
            assert_eq!(accept_uris(&mut host, &chosen), vec![expected]);
        },
    );
}

#[test]
fn confined_chooser_does_not_open_global_search() {
    run_chooser(
        "confined_chooser_does_not_open_global_search",
        "",
        |dir| {
            let root = dir.canonicalize().expect("canonical root");
            let kind = ChooserKind::Open {
                directory: true,
                multiple: false,
            };
            (chooser_request(root.clone(), kind, Vec::new()), Some(root))
        },
        |mut host, _chosen, _folder| {
            press(&mut host, Key::Text("k".into()), ctrl());
            assert!(
                !host.app().palette.state().is_open(),
                "a device-confined chooser cannot open global search"
            );
        },
    );
}
