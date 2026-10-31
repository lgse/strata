// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use futures_lite::future::block_on;

use crate::model::{ActionDefinition, ActionInput, MenuPlacement};

use super::*;

fn definition(id: &str, name: &str, enabled: bool, menu: &str) -> ActionDefinition {
    ActionDefinition::parse(&format!(
        "schema_version = 1\nid = \"{id}\"\nname = \"{name}\"\nenabled = {enabled}\nmenu = \"{menu}\"\n\n[when]\nextensions = [\"png\"]\n\n[run]\nruntime = \"command\"\nprogram = \"true\"\nargs = [\"{{paths}}\"]\n"
    ))
    .expect("test definition is valid")
}

fn handle(id: &str, name: &str, enabled: bool, menu: MenuPlacement) -> Rc<ActionHandle> {
    let menu = match menu {
        MenuPlacement::Top => "top",
        MenuPlacement::Submenu => "submenu",
    };
    handle_from(&definition(id, name, enabled, menu))
}

fn handle_from(definition: &ActionDefinition) -> Rc<ActionHandle> {
    Rc::new(ActionHandle {
        definition: definition.clone(),
        directory: PathBuf::from("/tmp/actions").join(&definition.id),
        availability: ActionAvailability::Available(ActionProgram::Command {
            program: OsString::from("/usr/bin/true"),
            arguments: Vec::new(),
        }),
    })
}

#[test]
fn matches_only_enabled_actions_whose_rules_accept_every_input() {
    let catalog = ActionCatalog::new(
        vec![
            handle("a1", "Zebra", true, MenuPlacement::Top),
            handle("a2", "alpha", true, MenuPlacement::Submenu),
            handle("a3", "Disabled", false, MenuPlacement::Top),
        ],
        Vec::new(),
    );
    let images = [ActionInput::file("a.png", Some("image/png"))];
    let matched = catalog.matches(&images);
    assert_eq!(
        matched
            .iter()
            .map(|action| action.action.name())
            .collect::<Vec<_>>(),
        vec!["alpha", "Zebra"],
        "results are ordered by name and skip disabled actions"
    );
    assert_eq!(matched[0].placement, MenuPlacement::Submenu);
    assert_eq!(matched[1].placement, MenuPlacement::Top);

    assert!(
        catalog
            .matches(&[ActionInput::file("a.txt", None)])
            .is_empty(),
        "conditions must reject non-matching inputs"
    );
    assert!(catalog.matches(&[]).is_empty());
}

#[test]
fn numbered_actions_follow_the_menu_and_stop_at_ten() {
    let mut actions = vec![
        handle("sub", "Aardvark", true, MenuPlacement::Submenu),
        handle("top-z", "Zebra", true, MenuPlacement::Top),
        handle("top-a", "Apple", true, MenuPlacement::Top),
        handle("off", "Aaa off", false, MenuPlacement::Top),
    ];
    let mut unavailable = (*handle("broken", "Aaa broken", true, MenuPlacement::Top)).clone();
    unavailable.availability = ActionAvailability::Unavailable {
        reason: "missing".to_owned(),
    };
    actions.push(Rc::new(unavailable));
    actions.extend((0..9).map(|index| {
        handle(
            &format!("more-{index}"),
            &format!("More {index}"),
            true,
            MenuPlacement::Submenu,
        )
    }));
    let catalog = ActionCatalog::new(actions, Vec::new());
    let images = [ActionInput::file("a.png", Some("image/png"))];

    let numbered: Vec<_> = catalog
        .numbered(&images)
        .iter()
        .map(|action| action.id().to_owned())
        .collect();

    let mut expected = vec!["top-a", "top-z", "sub"];
    let more: Vec<String> = (0..7).map(|index| format!("more-{index}")).collect();
    expected.extend(more.iter().map(String::as_str));
    assert_eq!(
        numbered, expected,
        "top level first, runnable only, ten at most"
    );
    assert!(
        catalog
            .numbered(&[ActionInput::file("a.txt", None)])
            .is_empty()
    );
}

#[test]
fn listed_actions_keep_their_place_and_run_only_where_they_apply() {
    let mut unavailable = (*handle("broken", "Broken", true, MenuPlacement::Top)).clone();
    unavailable.availability = ActionAvailability::Unavailable {
        reason: "missing".to_owned(),
    };
    let catalog = ActionCatalog::new(
        vec![
            handle("off", "Off", false, MenuPlacement::Top),
            Rc::new(unavailable),
            handle("zebra", "Zebra", true, MenuPlacement::Submenu),
        ],
        Vec::new(),
    );
    let images = [ActionInput::file("a.png", Some("image/png"))];
    let listed = |index| {
        catalog
            .listed(index, &images)
            .map(|action| action.id().to_owned())
    };
    assert_eq!(listed(0), None, "disabled");
    assert_eq!(listed(1), None, "unavailable");
    assert_eq!(
        listed(2).as_deref(),
        Some("zebra"),
        "the third listed, whatever its name"
    );
    assert_eq!(listed(3), None, "an empty slot");
    assert_eq!(catalog.listed(2, &[ActionInput::file("a.txt", None)]), None);
}

struct FakeStore {
    stored: Mutex<StoredActions>,
    writes: AtomicUsize,
    deletions: AtomicUsize,
    imports: AtomicUsize,
    exports: AtomicUsize,
    fail_write: AtomicBool,
    stalled: Mutex<Option<crate::test_support::Gate>>,
}

impl FakeStore {
    fn new(actions: Vec<Rc<ActionHandle>>) -> Arc<Self> {
        Arc::new(Self {
            stored: Mutex::new(StoredActions {
                actions: actions.iter().map(|action| (**action).clone()).collect(),
                failures: Vec::new(),
            }),
            writes: AtomicUsize::new(0),
            deletions: AtomicUsize::new(0),
            imports: AtomicUsize::new(0),
            exports: AtomicUsize::new(0),
            fail_write: AtomicBool::new(false),
            stalled: Mutex::new(None),
        })
    }

    fn stall(&self) -> crate::test_support::Gate {
        let gate = crate::test_support::Gate::default();
        self.stalled
            .lock()
            .expect("fake store")
            .replace(gate.clone());
        gate
    }

    fn reach_another_mount(&self) {
        let gate = self.stalled.lock().expect("fake store").clone();
        if let Some(gate) = gate {
            gate.racer()();
        }
    }

    fn set_actions(&self, actions: Vec<Rc<ActionHandle>>) {
        self.edit(|stored| {
            stored.actions = actions.iter().map(|action| (**action).clone()).collect();
        });
    }

    fn edit(&self, edit: impl FnOnce(&mut StoredActions)) {
        edit(&mut self.stored.lock().expect("fake store"));
    }
}

impl ActionStore for FakeStore {
    fn load(&self) -> StoredActions {
        self.stored.lock().expect("fake store").clone()
    }

    fn locate(&self, stored: StoredActions) -> StoredActions {
        self.reach_another_mount();
        stored
    }

    fn definition(&self, id: &str) -> Result<ActionDefinition, ActionStoreError> {
        self.load()
            .get(id)
            .map(|action| action.definition.clone())
            .ok_or_else(|| ActionStoreError::NotFound(id.to_owned()))
    }

    fn read(&self, id: &str) -> Result<ActionWriteRequest, ActionStoreError> {
        Ok(ActionWriteRequest {
            definition: self.definition(id)?,
            script: None,
        })
    }

    fn create(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError> {
        let id = &request.definition.id;
        if self.load().actions.iter().any(|action| action.id() == id) {
            return Err(ActionStoreError::AlreadyExists(id.clone()));
        }
        self.write(request)
    }

    fn write(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(ActionStoreError::Io("write refused".to_owned()));
        }
        self.edit(|stored| {
            stored
                .actions
                .retain(|action| action.id() != request.definition.id);
            stored
                .actions
                .push((*handle_from(&request.definition)).clone());
        });
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<(), ActionStoreError> {
        self.deletions.fetch_add(1, Ordering::SeqCst);
        self.edit(|stored| stored.actions.retain(|action| action.id() != id));
        Ok(())
    }

    fn read_import(&self, _source: &Path) -> Result<ActionWriteRequest, ActionStoreError> {
        self.reach_another_mount();
        Ok(ActionWriteRequest {
            definition: definition("imported", "Imported", false, "top"),
            script: None,
        })
    }

    fn import(&self, request: ActionWriteRequest) -> Result<String, ActionStoreError> {
        self.imports.fetch_add(1, Ordering::SeqCst);
        let id = request.definition.id.clone();
        self.create(&request)?;
        Ok(id)
    }

    fn export(
        &self,
        request: &ActionWriteRequest,
        destination: &Path,
    ) -> Result<PathBuf, ActionStoreError> {
        self.reach_another_mount();
        self.exports.fetch_add(1, Ordering::SeqCst);
        Ok(destination.join(&request.definition.id))
    }
}

fn write_request(id: &str) -> ActionWriteRequest {
    ActionWriteRequest {
        definition: definition(id, "Action", true, "top"),
        script: None,
    }
}

fn ids(catalog: &ActionCatalog) -> Vec<&str> {
    catalog.actions().iter().map(|action| action.id()).collect()
}

#[test]
fn registry_caches_until_something_changes() {
    let store = FakeStore::new(vec![handle("a1", "One", true, MenuPlacement::Top)]);
    let registry = ActionRegistry::new(store.clone());
    assert!(
        registry.catalog().actions().is_empty(),
        "nothing is read until a reload runs on the settings writer"
    );
    let first = block_on(registry.reload());
    assert!(
        Rc::ptr_eq(&first, &registry.catalog()),
        "the catalog stays cached"
    );

    let notifications = Rc::new(Cell::new(0));
    let counter = notifications.clone();
    let _observer = registry.observe(Rc::new(move || counter.set(counter.get() + 1)));
    assert!(Rc::ptr_eq(&first, &block_on(registry.reload())));
    assert_eq!(notifications.get(), 0);

    store.set_actions(vec![handle("a2", "Two", true, MenuPlacement::Top)]);
    let reloaded = block_on(registry.reload());
    assert_eq!(
        notifications.get(),
        1,
        "a changed catalog notifies observers"
    );
    assert_eq!(ids(&reloaded), vec!["a2"]);
}

#[test]
fn an_older_answer_never_replaces_a_newer_catalog() {
    let store = FakeStore::new(vec![handle("old", "Old", true, MenuPlacement::Top)]);
    let registry = ActionRegistry::new(store.clone());
    let older = registry.reload();
    let changed = store.clone();
    drop(crate::storage::writer::run(move || {
        changed.set_actions(vec![handle("new", "New", true, MenuPlacement::Top)]);
    }));
    let newer = registry.reload();

    assert_eq!(ids(&block_on(newer)), vec!["new"]);
    assert_eq!(ids(&block_on(older)), vec!["new"]);
}

#[test]
fn registry_delegates_edits_and_reloads() {
    let store = FakeStore::new(Vec::new());
    let registry = ActionRegistry::new(store.clone());
    block_on(registry.write(write_request("a1"))).expect("write succeeds");
    assert_eq!(store.writes.load(Ordering::SeqCst), 1);
    assert_eq!(
        registry.catalog().get("a1").map(|action| action.name()),
        Some("Action"),
        "a write refreshes the catalog with the stored definition"
    );

    assert_eq!(
        block_on(registry.export("a1".to_owned(), PathBuf::from("/tmp/destination")))
            .expect("export succeeds"),
        PathBuf::from("/tmp/destination/a1")
    );
    assert_eq!(store.exports.load(Ordering::SeqCst), 1);

    block_on(registry.delete("a1".to_owned())).expect("delete succeeds");
    assert_eq!(store.deletions.load(Ordering::SeqCst), 1);
    assert!(
        registry.catalog().get("a1").is_none(),
        "a delete refreshes the catalog"
    );
    assert_eq!(
        block_on(registry.import(PathBuf::from("/tmp/source"))).expect("import succeeds"),
        "imported"
    );
    assert_eq!(store.imports.load(Ordering::SeqCst), 1);
    assert!(registry.catalog().get("imported").is_some());
}

#[test]
fn a_failed_write_leaves_the_catalog_untouched() {
    let store = FakeStore::new(vec![handle("a1", "One", true, MenuPlacement::Top)]);
    let registry = ActionRegistry::new(store.clone());
    block_on(registry.reload());
    store.fail_write.store(true, Ordering::SeqCst);
    assert!(block_on(registry.write(write_request("a2"))).is_err());
    assert_eq!(
        ids(&registry.catalog()),
        vec!["a1"],
        "a refused write must not appear in the catalog"
    );
}

fn drive_until<F: Future>(future: &mut std::pin::Pin<Box<F>>, reached: &crate::test_support::Gate) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !reached.reached() {
        assert!(
            std::time::Instant::now() < deadline,
            "the stalled call starts"
        );
        assert!(
            block_on(futures_lite::future::poll_once(future.as_mut())).is_none(),
            "answered before reaching the other mount"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn writer_answers() -> bool {
    let (answered, answer) = std::sync::mpsc::channel();
    drop(crate::storage::writer::run(move || answered.send(()).ok()));
    answer
        .recv_timeout(std::time::Duration::from_secs(5))
        .is_ok()
}

#[test]
fn programs_and_picked_folders_never_hold_up_the_settings_writer() {
    let store = FakeStore::new(vec![handle("a1", "One", true, MenuPlacement::Top)]);
    let registry = ActionRegistry::new(store.clone());

    let gate = store.stall();
    let mut reloaded = Box::pin(registry.reload());
    drive_until(&mut reloaded, &gate);
    assert!(writer_answers(), "looking up programs");
    gate.release();
    assert_eq!(ids(&block_on(reloaded)), vec!["a1"]);

    let gate = store.stall();
    let mut imported = Box::pin(registry.import(PathBuf::from("/mnt/stopped/incoming")));
    drive_until(&mut imported, &gate);
    assert!(writer_answers(), "reading an imported folder");
    gate.release();
    assert_eq!(block_on(imported).expect("imported"), "imported");

    let gate = store.stall();
    let mut exported = Box::pin(registry.export("a1".to_owned(), PathBuf::from("/mnt/stopped")));
    drive_until(&mut exported, &gate);
    assert!(writer_answers(), "writing an exported folder");
    gate.release();
    assert_eq!(
        block_on(exported).expect("exported"),
        PathBuf::from("/mnt/stopped/a1")
    );
}
