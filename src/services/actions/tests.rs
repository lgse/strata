// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
};

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
fn catalog_lookup_is_by_id() {
    let catalog = ActionCatalog::new(
        vec![handle("a1", "One", true, MenuPlacement::Top)],
        Vec::new(),
    );
    assert_eq!(catalog.get("a1").map(|action| action.name()), Some("One"));
    assert!(catalog.get("missing").is_none());
}

struct FakeStore {
    catalog: RefCell<ActionCatalog>,
    writes: Cell<usize>,
    deletions: Cell<usize>,
    imports: Cell<usize>,
    exports: Cell<usize>,
    fail_write: Cell<bool>,
}

impl FakeStore {
    fn new(catalog: ActionCatalog) -> Rc<Self> {
        Rc::new(Self {
            catalog: RefCell::new(catalog),
            writes: Cell::new(0),
            deletions: Cell::new(0),
            imports: Cell::new(0),
            exports: Cell::new(0),
            fail_write: Cell::new(false),
        })
    }

    fn set_actions(&self, actions: Vec<Rc<ActionHandle>>) {
        let failures = self.catalog.borrow().failures().to_vec();
        self.catalog.replace(ActionCatalog::new(actions, failures));
    }
}

impl ActionStore for FakeStore {
    fn load(&self) -> ActionCatalog {
        self.catalog.borrow().clone()
    }

    fn write(&self, request: &ActionWriteRequest) -> Result<(), ActionStoreError> {
        self.writes.set(self.writes.get() + 1);
        if self.fail_write.get() {
            return Err(ActionStoreError::Io("write refused".to_owned()));
        }
        // Behave like the real store: the write becomes visible on the next load.
        let mut actions = self.catalog.borrow().actions().to_vec();
        actions.retain(|action| action.id() != request.definition.id);
        actions.push(handle_from(&request.definition));
        self.set_actions(actions);
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<(), ActionStoreError> {
        self.deletions.set(self.deletions.get() + 1);
        let mut actions = self.catalog.borrow().actions().to_vec();
        actions.retain(|action| action.id() != id);
        self.set_actions(actions);
        Ok(())
    }

    fn read_script(&self, _id: &str) -> Result<Option<ActionScript>, ActionStoreError> {
        Ok(None)
    }

    fn import(&self, _source: &Path) -> Result<String, ActionStoreError> {
        self.imports.set(self.imports.get() + 1);
        let mut actions = self.catalog.borrow().actions().to_vec();
        actions.push(handle("imported", "Imported", false, MenuPlacement::Top));
        self.set_actions(actions);
        Ok("imported".to_owned())
    }

    fn export(&self, id: &str, destination: &Path) -> Result<PathBuf, ActionStoreError> {
        self.exports.set(self.exports.get() + 1);
        Ok(destination.join(id))
    }
}

fn write_request(id: &str) -> ActionWriteRequest {
    ActionWriteRequest {
        definition: definition(id, "Action", true, "top"),
        script: None,
    }
}

#[test]
fn registry_caches_until_something_changes() {
    let store = FakeStore::new(ActionCatalog::new(
        vec![handle("a1", "One", true, MenuPlacement::Top)],
        Vec::new(),
    ));
    let registry = ActionRegistry::new(store.clone());
    let first = registry.catalog();
    assert!(
        Rc::ptr_eq(&first, &registry.catalog()),
        "the catalog stays cached"
    );

    // Reloading an unchanged store must not churn the catalog or notify.
    let notifications = Rc::new(Cell::new(0));
    let counter = notifications.clone();
    let _observer = registry.observe(Rc::new(move || counter.set(counter.get() + 1)));
    assert!(Rc::ptr_eq(&first, &registry.reload()));
    assert_eq!(notifications.get(), 0);

    store.set_actions(vec![handle("a2", "Two", true, MenuPlacement::Top)]);
    let reloaded = registry.reload();
    assert_eq!(
        notifications.get(),
        1,
        "a changed catalog notifies observers"
    );
    assert_eq!(
        reloaded
            .actions()
            .iter()
            .map(|action| action.id())
            .collect::<Vec<_>>(),
        vec!["a2"]
    );
}

#[test]
fn registry_delegates_edits_and_reloads() {
    let store = FakeStore::new(ActionCatalog::default());
    let registry = ActionRegistry::new(store.clone());
    registry
        .write(&write_request("a1"))
        .expect("write succeeds");
    assert_eq!(store.writes.get(), 1);
    assert_eq!(
        registry.catalog().get("a1").map(|action| action.name()),
        Some("Action"),
        "a write refreshes the catalog with the stored definition"
    );

    registry.delete("a1").expect("delete succeeds");
    assert_eq!(store.deletions.get(), 1);
    assert!(
        registry.catalog().get("a1").is_none(),
        "a delete refreshes the catalog"
    );
    assert_eq!(
        registry
            .import(Path::new("/tmp/source"))
            .expect("import succeeds"),
        "imported"
    );
    assert_eq!(store.imports.get(), 1);
    assert!(registry.catalog().get("imported").is_some());
    assert_eq!(
        registry
            .export("a1", Path::new("/tmp/destination"))
            .expect("export succeeds"),
        PathBuf::from("/tmp/destination/a1")
    );
    assert_eq!(store.exports.get(), 1);
}

#[test]
fn a_failed_write_leaves_the_catalog_untouched() {
    let store = FakeStore::new(ActionCatalog::new(
        vec![handle("a1", "One", true, MenuPlacement::Top)],
        Vec::new(),
    ));
    let registry = ActionRegistry::new(store.clone());
    store.fail_write.set(true);
    assert!(registry.write(&write_request("a2")).is_err());
    assert_eq!(
        registry
            .catalog()
            .actions()
            .iter()
            .map(|action| action.id())
            .collect::<Vec<_>>(),
        vec!["a1"],
        "a refused write must not appear in the catalog"
    );
}

#[test]
fn unloadable_definitions_are_reported_without_hiding_the_rest() {
    let failure = ActionLoadFailure {
        directory: "broken".to_owned(),
        error: ActionStoreError::Invalid(crate::model::ActionError::InvalidName),
    };
    let catalog = ActionCatalog::new(
        vec![handle("a1", "One", true, MenuPlacement::Top)],
        vec![failure.clone()],
    );
    assert_eq!(catalog.actions().len(), 1);
    assert_eq!(catalog.failures(), &[failure]);
    assert!(
        catalog.failures()[0].error.to_string().contains("name"),
        "the failure explains itself for Settings"
    );
}
