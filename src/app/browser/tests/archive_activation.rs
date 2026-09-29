// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn activating_recognized_local_archive_requests_extraction() {
    let browser = Browser::new(Rc::new(ArchiveFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));

    events.borrow_mut().clear();
    browser.activate(0, 0);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ExtractRequested { entry }
            if entry.location == Location::local("/fixture/archive.zip")
    )));

    events.borrow_mut().clear();
    browser.activate_in_place(0, 0);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ExtractRequested { entry }
            if entry.location == Location::local("/fixture/archive.zip")
    )));

    events.borrow_mut().clear();
    browser.select(0, 0);
    browser.activate_focused();
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ExtractRequested { entry }
            if entry.location == Location::local("/fixture/archive.zip")
    )));

    events.borrow_mut().clear();
    browser.select(0, 0);
    browser.activate_focused_in_place();
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::ExtractRequested { entry }
            if entry.location == Location::local("/fixture/archive.zip")
    )));
}

#[test]
fn activating_archive_in_chooser_mode_opens_for_selection() {
    let browser = Browser::new(Rc::new(ArchiveFileSource));
    browser.set_chooser_mode(true);
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));

    events.borrow_mut().clear();
    browser.activate(0, 0);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OpenRequested { location }
            if location == &Location::local("/fixture/archive.zip")
    )));
}

#[test]
fn activating_non_archive_file_or_remote_archive_opens_externally() {
    let browser = Browser::new(Rc::new(ArchiveFileSource));
    let events = Rc::new(RefCell::new(Vec::new()));
    let observed = events.clone();
    browser.observe(move |event| observed.borrow_mut().push(event.clone()));
    browser.navigate(Location::local("/fixture"));

    events.borrow_mut().clear();
    browser.activate(0, 1);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OpenRequested { location }
            if location == &Location::local("/fixture/notes.txt")
    )));

    events.borrow_mut().clear();
    browser.activate(0, 2);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        BrowserEvent::OpenRequested { location }
            if location == &Location::uri("sftp://example.com/remote-archive.zip")
    )));
}
