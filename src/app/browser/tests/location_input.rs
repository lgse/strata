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
