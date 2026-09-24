// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn valid_location_input_navigates_through_the_controller() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    assert_eq!(browser.navigate_input("/accepted"), Ok(()));

    assert_eq!(
        browser.active_location(),
        Some(Location::local("/accepted"))
    );
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ColumnAdded { depth: 0, location }
            if location == &Location::local("/accepted")
    )));
}

#[test]
fn location_input_expands_trimmed_home_relative_paths() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    let home = glib::home_dir();

    assert_eq!(browser.navigate_input("  ~  "), Ok(()));
    assert_eq!(browser.active_location(), Some(Location::local(&home)));

    assert_eq!(browser.navigate_input("  ~/Documents  "), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::local(home.join("Documents")))
    );
}

#[test]
fn home_relative_input_preserves_the_native_home_path() {
    let home = Path::new("/home/fixture");

    assert_eq!(
        location_from_input_with_home("~//Documents", home),
        Ok(Location::local("/home/fixture/Documents"))
    );
}

#[test]
fn typed_paths_drop_trailing_slashes_so_files_reveal() {
    let home = Path::new("/home/fixture");

    let file = location_from_input_with_home("/fixture/report.pdf/", home)
        .expect("a trailing slash is still a valid typed path");
    assert_eq!(
        file.native_path().map(Path::as_os_str),
        Some(OsStr::new("/fixture/report.pdf"))
    );

    let home_file = location_from_input_with_home("~/Documents/report.pdf/", home)
        .expect("a home-relative trailing slash is still valid");
    assert_eq!(
        home_file.native_path().map(Path::as_os_str),
        Some(OsStr::new("/home/fixture/Documents/report.pdf"))
    );

    let root = location_from_input_with_home("/", home).expect("the root stays the root");
    assert_eq!(
        root.native_path().map(Path::as_os_str),
        Some(OsStr::new("/"))
    );
}

#[test]
fn other_users_home_shorthand_is_rejected() {
    assert!(matches!(
        location_from_input_with_home("~other-user/Documents", Path::new("/home/fixture")),
        Err(LocationValidationError::UnsupportedShorthand(_))
    ));
}

#[test]
fn sidebar_location_navigation_validates_uris_but_navigates_native_paths_directly() {
    let remote_browser = Browser::new(Rc::new(NotMountedFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    remote_browser.observe(move |event| observed.borrow_mut().push(event.clone()));

    let remote = Location::uri("smb://host/share");
    remote_browser.navigate_location(remote.clone(), true);

    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LocationNavigationRejected {
            error: LocationValidationError::NotMounted(location)
        } if location == &remote
    )));
    assert_eq!(remote_browser.active_location(), None);

    let native_browser = Browser::new(Rc::new(RejectingFileSource));
    let native = Location::local("/saved/bookmark");
    native_browser.navigate_location(native.clone(), true);

    assert_eq!(native_browser.active_location(), Some(native));
}

#[test]
fn location_input_accepts_uri_schemes_for_local_and_remote_locations() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    assert_eq!(browser.navigate_input("smb://192.168.1.220/share"), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::uri("smb://192.168.1.220/share"))
    );

    assert_eq!(browser.navigate_input("sftp://user@host:2222/path"), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::uri("sftp://user@host:2222/path"))
    );

    assert_eq!(browser.navigate_input("/regular/absolute/path"), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::local("/regular/absolute/path"))
    );

    assert_eq!(browser.navigate_input("network:///"), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::uri("network:///"))
    );

    assert_eq!(browser.navigate_input("recent:///"), Ok(()));
    assert_eq!(browser.active_location(), Some(Location::uri("recent:///")));
    assert_eq!(browser.navigate_input("recent://"), Ok(()));
    assert!(
        browser
            .active_location()
            .is_some_and(|location| location.is_recent_root())
    );
}

#[test]
fn location_input_rejects_unsupported_uri_schemes() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    for uri in [
        "https://example.com/files",
        "file:///tmp",
        "custom://host/path",
    ] {
        assert!(matches!(
            browser.navigate_input(uri),
            Err(LocationValidationError::UnsupportedScheme(_))
        ));
        assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    }

    assert_eq!(browser.navigate_input("SMB://host/share"), Ok(()));
    assert_eq!(
        browser.active_location(),
        Some(Location::uri("smb://host/share"))
    );
}

#[test]
fn location_input_rejects_unc_and_scp_shorthand_with_a_helpful_message() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    for shorthand in [
        r"\\host\share",
        r"smb:\\192.168.1.220",
        "//host/share",
        "//192.168.1.220",
        "user@host:path",
    ] {
        assert!(matches!(
            browser.navigate_input(shorthand),
            Err(LocationValidationError::UnsupportedShorthand(_))
        ));
        assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    }
}

#[test]
fn location_input_rejects_uris_with_an_embedded_password() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    for uri in [
        "smb://user:secret@host/share",
        "smb://user;password=secret@host/share",
    ] {
        assert_eq!(
            browser.navigate_input(uri),
            Err(LocationValidationError::EmbeddedCredential)
        );
        assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    }
}

#[test]
fn location_input_reports_the_target_location_when_not_mounted() {
    let browser = Browser::new(Rc::new(NotMountedFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    assert_eq!(browser.navigate_input("smb://192.168.1.220/share"), Ok(()));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LocationNavigationRejected {
            error: LocationValidationError::NotMounted(location)
        } if location == &Location::uri("smb://192.168.1.220/share")
    )));
    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
}

#[test]
fn descending_into_an_unmounted_location_reports_it_for_retry() {
    let browser = Browser::new(Rc::new(NotMountedFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.descend(0, Location::uri("smb://192.168.1.220/share"));

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::NavigationRejected {
            parent_depth: 0,
            error: LocationValidationError::NotMounted(location)
        } if location == &Location::uri("smb://192.168.1.220/share")
    )));
}

#[test]
fn rejected_directory_activation_preserves_navigation_state() {
    let browser = Browser::new(Rc::new(RejectingFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    browser.descend(0, Location::local("/fixture/restricted"));

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert!(
        events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::NavigationRejected { .. }))
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|event| matches!(event, BrowserEvent::ColumnAdded { depth: 1, .. }))
    );
}

#[test]
fn rejected_location_input_preserves_navigation_state() {
    let browser = Browser::new(Rc::new(RejectingFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    assert_eq!(
        browser.navigate_input("/restricted"),
        Err(LocationValidationError::Inaccessible)
    );

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert!(events.borrow().is_empty());
}

#[test]
fn invalid_location_text_is_rejected_before_the_provider() {
    let browser = Browser::new(Rc::new(FakeFileSource));
    browser.navigate(Location::local("/fixture"));

    assert_eq!(
        browser.navigate_input(""),
        Err(LocationValidationError::Empty)
    );
    assert_eq!(
        browser.navigate_input("   "),
        Err(LocationValidationError::Empty)
    );
    assert_eq!(
        browser.navigate_input("relative/path"),
        Err(LocationValidationError::NotAbsolute)
    );
    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
}

struct TypedPathSource {
    files: HashSet<Location>,
    errors: HashMap<Location, LocationValidationError>,
    listing: Vec<FileEntry>,
}

impl TypedPathSource {
    fn containing(file: Location, listing: Vec<FileEntry>) -> Self {
        Self {
            files: HashSet::from([file]),
            errors: HashMap::new(),
            listing,
        }
    }
}

impl FileSource for TypedPathSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if let Some(error) = self.errors.get(location) {
            return Err(error.clone());
        }
        if self.files.contains(location) {
            return Err(LocationValidationError::NotDirectory);
        }
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if self
            .files
            .iter()
            .any(|file| file.parent().as_ref() == Some(&request.location))
        {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries: self.listing.clone(),
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

struct RefreshingTypedPathSource {
    file: Location,
    enumerations: Cell<usize>,
    initial_listing: Vec<FileEntry>,
    refreshed_listing: Vec<FileEntry>,
}

type ValidationEmit = Rc<dyn Fn(Result<(), LocationValidationError>)>;

struct NestedValidationSource {
    file: Location,
    parent_validation: RefCell<Option<ValidationEmit>>,
    parent_cancelled: Rc<Cell<bool>>,
}

impl FileSource for NestedValidationSource {
    fn validate_location(&self, _location: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), LocationValidationError>)>,
    ) -> LoadHandle {
        if location == self.file {
            emit(Err(LocationValidationError::NotDirectory));
            return LoadHandle::new(|| {});
        }
        self.parent_validation.replace(Some(emit));
        let cancelled = self.parent_cancelled.clone();
        LoadHandle::new(move || cancelled.set(true))
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

impl FileSource for RefreshingTypedPathSource {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        if location == &self.file {
            Err(LocationValidationError::NotDirectory)
        } else {
            Ok(())
        }
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        let enumeration = self.enumerations.get();
        self.enumerations.set(enumeration + 1);
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: if enumeration == 0 {
                self.initial_listing.clone()
            } else {
                self.refreshed_listing.clone()
            },
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: None,
            can_delete: None,
        });
        LoadHandle::new(|| {})
    }
}

fn listing_file(location: Location, name: &str, is_hidden: bool) -> FileEntry {
    FileEntry {
        location,
        native_name: OsString::from(name),
        thumbnail_path: None,
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        is_hidden,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
    }
}

fn selected_locations(browser: &Browser) -> Vec<Location> {
    browser
        .selected_entries()
        .iter()
        .map(|entry| entry.location.clone())
        .collect()
}

#[test]
fn location_input_naming_a_file_reveals_it_inside_its_parent() {
    let file = Location::local("/fixture/report.pdf");
    let browser = Browser::new(Rc::new(TypedPathSource::containing(
        file.clone(),
        vec![
            listing_file(Location::local("/fixture/notes.txt"), "notes.txt", false),
            listing_file(file.clone(), "report.pdf", false),
        ],
    )));
    browser.navigate(Location::local("/start"));

    assert_eq!(browser.navigate_input("/fixture/report.pdf"), Ok(()));

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert_eq!(selected_locations(&browser), vec![file]);
}

#[test]
fn location_input_naming_a_remote_file_reveals_it_inside_its_parent() {
    let file = Location::uri("sftp://host/share/report.pdf");
    let browser = Browser::new(Rc::new(TypedPathSource::containing(
        file.clone(),
        vec![
            listing_file(
                Location::uri("sftp://host/share/notes.txt"),
                "notes.txt",
                false,
            ),
            listing_file(file.clone(), "report.pdf", false),
        ],
    )));
    browser.navigate(Location::local("/start"));

    assert_eq!(
        browser.navigate_input("sftp://host/share/report.pdf"),
        Ok(())
    );

    assert_eq!(
        browser.active_location(),
        Some(Location::uri("sftp://host/share"))
    );
    assert_eq!(selected_locations(&browser), vec![file]);
}

#[test]
fn synchronous_file_validation_keeps_async_parent_validation_alive() {
    let file = Location::uri("sftp://host/share/report.pdf");
    let source = Rc::new(NestedValidationSource {
        file: file.clone(),
        parent_validation: RefCell::new(None),
        parent_cancelled: Rc::new(Cell::new(false)),
    });
    let browser = Browser::new(source.clone());
    browser.navigate(Location::local("/start"));

    assert_eq!(
        browser.navigate_input("sftp://host/share/report.pdf"),
        Ok(())
    );

    assert!(source.parent_validation.borrow().is_some());
    assert!(!source.parent_cancelled.get());
}

#[test]
fn location_input_naming_a_file_inside_the_open_directory_selects_it_in_place() {
    let file = Location::local("/fixture/report.pdf");
    let browser = Browser::new(Rc::new(TypedPathSource::containing(
        file.clone(),
        vec![
            listing_file(Location::local("/fixture/notes.txt"), "notes.txt", false),
            listing_file(file.clone(), "report.pdf", false),
        ],
    )));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));
    events.borrow_mut().clear();

    assert_eq!(browser.navigate_input("/fixture/report.pdf"), Ok(()));

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert_eq!(selected_locations(&browser), vec![file]);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::SelectionSetChanged {
            take_focus: true,
            ..
        }
    )));
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::Reset | BrowserEvent::ColumnAdded { .. }
    )));
}

#[test]
fn location_input_uses_loaded_metadata_to_reveal_hidden_files() {
    let file = Location::local("/fixture/secret");
    let browser = Browser::new(Rc::new(TypedPathSource::containing(
        file.clone(),
        vec![listing_file(file.clone(), "secret", true)],
    )));
    browser.navigate(Location::local("/start"));
    assert!(!browser.preferences().show_hidden);

    assert_eq!(browser.navigate_input("/fixture/secret"), Ok(()));

    assert!(browser.preferences().show_hidden);
    assert_eq!(selected_locations(&browser), vec![file]);
}

#[test]
fn location_input_refreshes_an_open_parent_to_reveal_a_new_target() {
    let file = Location::local("/fixture/report.pdf");
    let notes = listing_file(Location::local("/fixture/notes.txt"), "notes.txt", false);
    let source = Rc::new(RefreshingTypedPathSource {
        file: file.clone(),
        enumerations: Cell::new(0),
        initial_listing: vec![notes.clone()],
        refreshed_listing: vec![notes, listing_file(file.clone(), "report.pdf", false)],
    });
    let browser = Browser::new(source.clone());
    browser.navigate(Location::local("/fixture"));
    assert_eq!(source.enumerations.get(), 1);

    assert_eq!(browser.navigate_input("/fixture/report.pdf"), Ok(()));

    assert_eq!(source.enumerations.get(), 2);
    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert_eq!(selected_locations(&browser), vec![file]);
}

#[test]
fn location_input_reports_when_the_named_file_is_missing_from_the_loaded_parent() {
    let file = Location::local("/fixture/report.pdf");
    let browser = Browser::new(Rc::new(TypedPathSource::containing(
        file.clone(),
        vec![listing_file(
            Location::local("/fixture/notes.txt"),
            "notes.txt",
            false,
        )],
    )));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/start"));
    events.borrow_mut().clear();

    assert_eq!(browser.navigate_input("/fixture/report.pdf"), Ok(()));

    assert_eq!(browser.active_location(), Some(Location::local("/fixture")));
    assert!(selected_locations(&browser).is_empty());
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LocationRevealFailed { location } if location == &file
    )));
}

#[test]
fn location_input_naming_a_file_reports_a_rejected_parent() {
    let file = Location::local("/fixture/report.pdf");
    let mut source = TypedPathSource::containing(
        file.clone(),
        vec![listing_file(file.clone(), "report.pdf", false)],
    );
    source.errors.insert(
        Location::local("/fixture"),
        LocationValidationError::Inaccessible,
    );
    let browser = Browser::new(Rc::new(source));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/start"));
    events.borrow_mut().clear();

    assert_eq!(browser.navigate_input("/fixture/report.pdf"), Ok(()));

    assert_eq!(browser.active_location(), Some(Location::local("/start")));
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::LocationNavigationRejected {
            error: LocationValidationError::Inaccessible
        }
    )));
}
