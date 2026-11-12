// SPDX-License-Identifier: MIT

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use super::*;

#[derive(Debug, Clone, Copy)]
enum Departure {
    Delete,
    Rename,
    MoveIntoAnotherFolder,
    /// Followed by a write into the open folder, the first change its monitor sees.
    RenameParent,
    DeleteParent,
    /// The removal of the open child column and of the folder share one monitor batch.
    DeleteWithAnOpenChildColumn,
    /// More entries than a monitor batch holds, so the monitor asks for a rescan.
    DeleteAFullFolder,
    /// Deleted and created again before the report arrives, as a build does to `dist/`.
    Recreate,
}

fn pump_for(condition: &dyn Fn() -> bool) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() && Instant::now() < deadline {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn load_finished(events: &RefCell<Vec<BrowserEvent>>, depth: usize) -> bool {
    any_event(
        events,
        |event| matches!(event, BrowserEvent::LoadFinished { depth: finished, .. } if *finished == depth),
    )
}

/// `parent/open` holding a file and an inner folder, beside `parent/elsewhere`.
struct Tree {
    fixture: tempfile::TempDir,
    parent: PathBuf,
    open: PathBuf,
}

impl Tree {
    fn new(departure: Departure) -> Self {
        let fixture = tempfile::tempdir().expect("fixture");
        let parent = fixture.path().join("parent");
        let open = parent.join("open");
        std::fs::create_dir_all(open.join("inner")).expect("open folder");
        std::fs::write(open.join("kept.txt"), "kept").expect("file");
        std::fs::create_dir(parent.join("elsewhere")).expect("sibling folder");
        if matches!(departure, Departure::DeleteAFullFolder) {
            for index in 0..5_000 {
                std::fs::write(open.join(format!("file-{index}")), "").expect("file");
            }
        }
        Self {
            fixture,
            parent,
            open,
        }
    }

    fn delete_open(&self) -> PathBuf {
        std::fs::remove_dir_all(&self.open).expect("delete");
        self.parent.clone()
    }

    fn recreate_open(&self) -> PathBuf {
        std::fs::remove_dir_all(&self.open).expect("delete");
        std::fs::create_dir(&self.open).expect("recreate");
        std::fs::write(self.open.join("fresh.txt"), "fresh").expect("file");
        self.open.clone()
    }
}

impl Departure {
    /// Changes the tree and returns the folder the browser should end up on.
    fn apply(self, tree: &Tree) -> PathBuf {
        match self {
            Self::Delete | Self::DeleteWithAnOpenChildColumn | Self::DeleteAFullFolder => {
                tree.delete_open()
            }
            Self::Rename => {
                let renamed = tree.parent.join("open-renamed");
                std::fs::rename(&tree.open, &renamed).expect("rename");
                renamed
            }
            Self::MoveIntoAnotherFolder => {
                std::fs::rename(&tree.open, tree.parent.join("elsewhere").join("open"))
                    .expect("move");
                tree.parent.clone()
            }
            Self::RenameParent => {
                let renamed = tree.fixture.path().join("parent-renamed");
                std::fs::rename(&tree.parent, &renamed).expect("rename parent");
                let open = renamed.join("open");
                std::fs::write(open.join("two.txt"), "2").expect("file");
                open
            }
            Self::DeleteParent => {
                std::fs::remove_dir_all(&tree.parent).expect("delete parent");
                tree.fixture.path().to_path_buf()
            }
            Self::Recreate => tree.recreate_open(),
        }
    }
}

fn open_with_its_identity(tree: &Tree, departure: Departure) -> (Rc<Browser>, Events) {
    let browser = Browser::new(Rc::new(crate::adapters::LocalFileSource));
    let events = recorded_events(&browser);
    browser.navigate(Location::local(&tree.open));
    pump_for(&|| load_finished(&events, 0) && browser.root_identity.borrow().is_some());
    if matches!(departure, Departure::DeleteWithAnOpenChildColumn) {
        browser.show_child(0, Location::local(tree.open.join("inner")));
        pump_for(&|| load_finished(&events, 1));
    }
    (browser, events)
}

fn the_recreated_folder_keeps_listing(browser: &Browser, tree: &Tree) -> Option<String> {
    pump_for(&|| column_names(browser, 0) == ["fresh.txt"]);
    std::fs::write(tree.open.join("later.txt"), "later").expect("file");
    pump_for(&|| column_names(browser, 0) == ["fresh.txt", "later.txt"]);
    let names = column_names(browser, 0);
    (names != ["fresh.txt", "later.txt"]).then(|| {
        format!(
            "{:?}: the reloaded listing shows {names:?}",
            Departure::Recreate
        )
    })
}

fn follows_the_departure(departure: Departure) -> Vec<String> {
    let tree = Tree::new(departure);
    let (browser, _events) = open_with_its_identity(&tree, departure);
    let could_go_back = browser.can_go_back();

    let expected = Location::local(departure.apply(&tree));
    pump_for(&|| browser.location_at(0).as_ref() == Some(&expected));
    let mut failures = Vec::new();
    if matches!(departure, Departure::Recreate) {
        failures.extend(the_recreated_folder_keeps_listing(&browser, &tree));
    }
    if matches!(departure, Departure::RenameParent) {
        let listed = ["inner", "kept.txt", "two.txt"];
        pump_for(&|| column_names(&browser, 0) == listed);
        let names = column_names(&browser, 0);
        if names != listed {
            failures.push(format!(
                "{departure:?}: the followed folder lists {names:?}"
            ));
        }
    }
    let path = open_path(&browser);
    if (&path, browser.can_go_back()) != (&vec![expected.clone()], could_go_back) {
        failures.push(format!(
            "{departure:?}: expected [{:?}], got {:?}; can_go_back {could_go_back}->{}",
            expected.display_name(),
            path.iter().map(Location::display_name).collect::<Vec<_>>(),
            browser.can_go_back(),
        ));
    }
    failures
}

#[test]
fn the_open_directory_follows_an_outside_rename_and_leaves_an_outside_removal() {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("async test lock");
    let failures: Vec<String> = [
        Departure::Delete,
        Departure::Rename,
        Departure::MoveIntoAnotherFolder,
        Departure::RenameParent,
        Departure::DeleteParent,
        Departure::DeleteWithAnOpenChildColumn,
        Departure::DeleteAFullFolder,
        Departure::Recreate,
    ]
    .into_iter()
    .flat_map(follows_the_departure)
    .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
