// SPDX-License-Identifier: MIT

use super::{location_input::listing_file, *};
use crate::{
    model::hidden_names::Leftover,
    services::file_activity::{self, OperationScope},
};

const UUID: &str = "0b4c7e1a-93f2-4d6e-8a51-2f9c0d7b6e34";

#[derive(Default)]
struct Listings(RefCell<HashMap<Location, Vec<FileEntry>>>);

impl FileSource for Listings {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if let Some(entries) = self.0.borrow().get(&request.location).cloned() {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries,
            });
        }
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

fn folder(path: &str) -> Location {
    if path.contains("://") {
        Location::uri(path)
    } else {
        Location::local(path)
    }
}

fn listed(folder: &Location, name: &str) -> FileEntry {
    let location = folder.child(OsStr::new(name)).expect("a child location");
    let mut entry = listing_file(location, name, name.starts_with('.'));
    if !name.contains('.') || name.starts_with(".strata-originals-") {
        entry.kind = EntryKind::Directory;
    }
    entry
}

fn browsing(listings: &[(&Location, &[String])]) -> (Rc<Browser>, Rc<RefCell<Vec<BrowserEvent>>>) {
    let source = Listings::default();
    for (folder, names) in listings {
        source.0.borrow_mut().insert(
            (*folder).clone(),
            names.iter().map(|name| listed(folder, name)).collect(),
        );
    }
    let browser = Browser::new(Rc::new(source));
    browser.set_operation_provider(Rc::new(ImmediateOperationProvider));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    (browser, events)
}

fn announced(events: &Rc<RefCell<Vec<BrowserEvent>>>) -> Vec<(usize, bool)> {
    events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            BrowserEvent::LeftoversChanged { depth, announce } => Some((*depth, *announce)),
            _ => None,
        })
        .collect()
}

fn surfaced(browser: &Browser, depth: usize) -> Vec<(String, Leftover)> {
    browser
        .column_leftovers(depth)
        .into_iter()
        .map(|(entry, kind)| (entry.display_name, kind))
        .collect()
}

fn hidden(prefix: &str) -> String {
    format!("{prefix}{UUID}")
}

#[test]
fn leftovers_wait_for_an_operation_in_their_folder_only() {
    let mut failures = Vec::new();
    for (row, base, writes_into, renames, during, after) in [
        (
            "writing into the parent",
            "/leftovers-wait",
            Some(0),
            None,
            vec![(1, false)],
            vec![(0, true)],
        ),
        (
            "writing into the child",
            "/leftovers-wait",
            Some(1),
            None,
            vec![(0, true)],
            vec![(1, false)],
        ),
        (
            "renaming the parent",
            "/leftovers-wait",
            None,
            Some(0),
            vec![],
            vec![(0, true), (1, false)],
        ),
        (
            "writing into a GVfs parent",
            "sftp://host/leftovers-wait",
            Some(0),
            None,
            vec![(1, false)],
            vec![(0, true)],
        ),
    ] {
        let parent = folder(&format!("{base}/{}", row.replace(' ', "-")));
        let child = parent.child(OsStr::new("child")).expect("child");
        let originals = hidden(".strata-originals-");
        let stage = hidden(".strata-copy-");
        let (browser, events) = browsing(&[
            (
                &parent,
                &[originals.clone(), "notes.txt".into(), "child".into()],
            ),
            (&child, std::slice::from_ref(&stage)),
        ]);
        let columns = [parent.clone(), child.clone()];
        let path = |depth: Option<usize>| {
            depth
                .map(|depth: usize| columns[depth].native_path().expect("native").to_path_buf())
                .into_iter()
                .collect()
        };
        let activity = file_activity::begin(OperationScope {
            roots: path(renames),
            folders: Vec::new(),
        });
        if let Some(depth) = writes_into {
            activity.include_folder(&columns[depth]);
        }
        let shown = |depths: &[(usize, bool)]| -> Vec<Vec<(String, Leftover)>> {
            let all = [
                vec![(originals.clone(), Leftover::Originals)],
                vec![(stage.clone(), Leftover::IncompleteCopy)],
            ];
            (0..2)
                .map(|depth| {
                    if depths.iter().any(|(shown, _)| *shown == depth) {
                        all[depth].clone()
                    } else {
                        Vec::new()
                    }
                })
                .collect()
        };

        browser.navigate(parent.clone());
        browser.show_child(0, child.clone());
        let busy = || {
            columns
                .iter()
                .map(Browser::leftovers_busy)
                .collect::<Vec<_>>()
        };
        let got_during = (
            announced(&events),
            vec![surfaced(&browser, 0), surfaced(&browser, 1)],
            busy(),
        );
        events.borrow_mut().clear();
        drop(activity);
        let got_after = (
            announced(&events),
            vec![surfaced(&browser, 0), surfaced(&browser, 1)],
            busy(),
        );

        let busy_during = (0..2)
            .map(|depth| !during.iter().any(|(shown, _)| *shown == depth))
            .collect();
        let want_during = (during.clone(), shown(&during), busy_during);
        let want_after = (after, shown(&[(0, true), (1, false)]), vec![false, false]);
        if got_during != want_during || got_after != want_after {
            failures.push(format!(
                "{row}: during {got_during:?}, want {want_during:?}; after {got_after:?}, want {want_after:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn peek_loads_announce_nothing_while_a_committed_load_does() {
    let parent = folder("/leftovers-peek");
    let child = parent.child(OsStr::new("album")).expect("child");
    let merge = hidden(".strata-merge-");
    let (browser, events) = browsing(&[
        (&parent, &["album".into()]),
        (&child, &[merge.clone(), "track.flac".into()]),
    ]);
    browser.navigate(parent);
    let before = announced(&events);

    browser.begin_peek(0, child.clone());
    let peeked = announced(&events);
    browser.close_peek();
    browser.show_child(0, child);

    assert_eq!((before, peeked), (Vec::new(), Vec::new()));
    assert_eq!(announced(&events), [(1, true)]);
    assert_eq!(surfaced(&browser, 1), [(merge, Leftover::MergeOriginals)]);
}

#[test]
fn listed_names_surface_only_what_interrupted_operations_left_where_someone_browses() {
    let probe = hidden(".strata-probe-");
    let stage = hidden(".strata-copy-");
    let replaced = hidden(".strata-replaced-");
    let legacy = ".strata-replacement-aB3xY9".to_owned();
    let names = [
        probe.clone(),
        stage.clone(),
        replaced.clone(),
        legacy.clone(),
        ".nfs000000000000001a00000002".to_owned(),
        ".fuse_hidden0000000a00000001".to_owned(),
        ".__smbfile_silly1f".to_owned(),
        ".strata-copy-notes".to_owned(),
        "report.txt".to_owned(),
    ];
    let everything = vec![
        (stage.clone(), Leftover::IncompleteCopy),
        (replaced.clone(), Leftover::Original),
        (legacy.clone(), Leftover::MaybeOriginal),
    ];
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    let mut failures = Vec::new();
    for (row, path, chooser, resorted, expected, swept) in [
        (
            "a local folder",
            "/leftovers-listed/local",
            false,
            false,
            true,
            true,
        ),
        (
            "a local folder sorted again",
            "/leftovers-listed/resorted",
            false,
            true,
            true,
            true,
        ),
        (
            "a GVfs folder",
            "sftp://host/leftovers-listed",
            false,
            false,
            true,
            false,
        ),
        (
            "the chooser",
            "/leftovers-listed/chooser",
            true,
            false,
            false,
            false,
        ),
        (
            "Trash",
            "trash:///leftovers-listed",
            false,
            false,
            false,
            false,
        ),
        (
            "Recent",
            "recent:///leftovers-listed",
            false,
            false,
            false,
            false,
        ),
    ] {
        let location = folder(path);
        let (browser, events) = browsing(&[(&location, &names)]);
        browser.set_chooser_mode(chooser);
        SWEPT_PROBES.with_borrow_mut(Vec::clear);

        browser.navigate(location.clone());
        if resorted {
            browser.set_sort_direction(0, SortDirection::Descending);
            pump_until(|| {
                events
                    .borrow()
                    .iter()
                    .any(|event| matches!(event, BrowserEvent::SortingFinished { depth: 0 }))
                    && browser.sort_resync_settled()
            });
        }

        let mut shown = surfaced(&browser, 0);
        shown.sort_by(|left, right| left.0.cmp(&right.0));
        let got = (
            shown,
            announced(&events),
            SWEPT_PROBES.with_borrow_mut(std::mem::take),
        );
        let want = (
            if expected {
                everything.clone()
            } else {
                Vec::new()
            },
            if expected {
                vec![(0, true)]
            } else {
                Vec::new()
            },
            if swept {
                vec![(location, vec![OsString::from(&probe)])]
            } else {
                Vec::new()
            },
        );
        if got != want {
            failures.push(format!("{row}: got {got:?}, want {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_folder_holding_originals_is_announced_once_per_session() {
    let originals = folder("/leftovers-once/originals");
    let copies = folder("/leftovers-once/copies");
    let elsewhere = folder("/leftovers-once/elsewhere");
    let (browser, events) = browsing(&[
        (&originals, &[hidden(".strata-rename-")]),
        (&copies, &[hidden(".strata-extraction-")]),
        (&elsewhere, &["notes.txt".into()]),
    ]);

    browser.navigate(originals.clone());
    browser.navigate(elsewhere);
    browser.navigate(originals);
    browser.navigate(copies);

    assert_eq!(announced(&events), [(0, true), (0, false), (0, false)]);
}
