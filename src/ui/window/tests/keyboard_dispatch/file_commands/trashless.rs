// SPDX-License-Identifier: MIT

use super::*;

/// Local browsing whose listings report that Trash is unsupported, like a
/// tmpfs, or, when `read_only`, a folder the user cannot delete from either.
struct TrashlessLocal {
    read_only: bool,
}

impl crate::services::FileSource for TrashlessLocal {
    fn allows_entry(&self, entry: &crate::model::FileEntry) -> bool {
        crate::adapters::LocalFileSource.allows_entry(entry)
    }

    fn validate_location(
        &self,
        location: &Location,
    ) -> Result<(), crate::services::LocationValidationError> {
        crate::adapters::LocalFileSource.validate_location(location)
    }

    fn validate_location_async(
        &self,
        location: Location,
        emit: Rc<dyn Fn(Result<(), crate::services::LocationValidationError>)>,
    ) -> crate::services::LoadHandle {
        crate::adapters::LocalFileSource.validate_location_async(location, emit)
    }

    fn enumerate(
        &self,
        request: crate::services::DirectoryRequest,
        emit: Rc<dyn Fn(crate::services::DirectoryEvent)>,
    ) -> crate::services::LoadHandle {
        let read_only = self.read_only;
        crate::adapters::LocalFileSource.enumerate(
            request,
            Rc::new(move |event| emit(without_trash(event, read_only))),
        )
    }

    fn supports_metadata_fill(&self, location: &Location) -> bool {
        crate::adapters::LocalFileSource.supports_metadata_fill(location)
    }

    fn fill_metadata(
        &self,
        request: crate::services::MetadataRequest,
        emit: Rc<dyn Fn(crate::services::DirectoryEvent)>,
    ) -> crate::services::LoadHandle {
        crate::adapters::LocalFileSource.fill_metadata(request, emit)
    }

    fn start_watch(
        &self,
        request: crate::services::WatchRequest,
        notify: Rc<dyn Fn(crate::services::DirectoryChange)>,
        report: Rc<dyn Fn(crate::services::Watching)>,
    ) -> crate::services::LoadHandle {
        crate::adapters::LocalFileSource.start_watch(request, notify, report)
    }
}

fn without_trash(
    event: crate::services::DirectoryEvent,
    read_only: bool,
) -> crate::services::DirectoryEvent {
    match event {
        crate::services::DirectoryEvent::Finished {
            request_id,
            truncated,
            can_delete,
            ..
        } => crate::services::DirectoryEvent::Finished {
            request_id,
            truncated,
            can_trash: Some(false),
            can_delete: if read_only { Some(false) } else { can_delete },
        },
        event => event,
    }
}

fn widget_contains_text(widget: &gtk::Widget, text: &str) -> bool {
    let label_matches = widget.downcast_ref::<gtk::Label>().is_some_and(|label| {
        label
            .text()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains(text)
    });
    label_matches
        || std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
            .any(|child| widget_contains_text(&child, text))
}

fn modal_text_contains(fixture: &KeyboardFixture, text: &str) -> bool {
    widget_with_class(fixture.overlay.upcast_ref(), "app-modal-layer")
        .is_some_and(|layer| widget_contains_text(&layer, text))
}

#[derive(Clone, Copy)]
struct TrashlessDelete {
    read_only: bool,
    tenxer: bool,
    key: Key,
}

fn press_delete_on_a(fixture: &KeyboardFixture, key: Key) {
    focus_files(fixture);
    move_to_named(fixture, &fixture.view.browser(), "a.txt");
    plain(fixture, key);
}

/// The folder refuses deletion too, so a missing Trash is not the whole story
/// and the trash attempt reports the error.
fn read_only_keeps_the_trash_attempt(fixture: &KeyboardFixture, held: &HeldOperations) {
    let id = fixture
        .view
        .browser()
        .last_started_operation()
        .expect("trash attempt");
    assert_eq!(
        (
            held.delete_requests.borrow().as_slice(),
            modal_visible(&fixture.overlay)
        ),
        ([(id, false, 1)].as_slice(), false),
        "a read-only folder keeps the trash attempt: (delete requests, dialog open)"
    );
}

fn without_trash_cancel_is_focused_and_enter_cancels(
    fixture: &KeyboardFixture,
    held: &HeldOperations,
    case: TrashlessDelete,
) {
    let TrashlessDelete { tenxer, key, .. } = case;
    assert_eq!(
        (
            held.delete_requests.borrow().len(),
            modal_visible(&fixture.overlay),
            modal_text_contains(fixture, "Move to Trash"),
            modal_text_contains(fixture, "doesn't support Trash"),
        ),
        (0, true, false, true),
        "{:?} (tenxer: {tenxer}): (delete requests started, dialog open, \
         is the Move to Trash dialog, explains missing Trash)",
        key.name()
    );
    wait_focused_button(fixture, "Cancel");
    pump(300);
    assert_eq!(
        focused_button(fixture).and_then(|button| button.label()),
        Some("Cancel".into()),
        "the size summary must not move focus to Permanently delete"
    );
    if tenxer {
        modal_key(fixture, Key::d);
        pump(100);
        assert!(modal_visible(&fixture.overlay), "d never confirms it");
        assert!(held.delete_requests.borrow().is_empty());
    }
    assert!(modal_key(fixture, Key::Return));
    wait_until(|| !modal_visible(&fixture.overlay));
    assert!(
        fixture._directory.path().join("a.txt").exists(),
        "Enter cancels"
    );
    assert!(held.delete_requests.borrow().is_empty());
}

fn confirm_deletes_permanently(fixture: &KeyboardFixture, held: &HeldOperations, key: Key) {
    press_delete_on_a(fixture, key);
    wait_confirm_sensitive(fixture);
    assert!(click_class(&fixture.overlay, "action-dialog-confirm"));
    wait_until(|| !held.delete_requests.borrow().is_empty());
    let (id, permanent, count) = held.delete_requests.borrow_mut().remove(0);
    assert_eq!(
        (permanent, count),
        (true, 1),
        "{key:?} confirms permanently"
    );
    held.emit(
        id,
        crate::services::OperationEvent::Deleted {
            request_id: id,
            locations: vec![Location::local(fixture._directory.path().join("a.txt"))],
        },
    );
    wait_until(|| !modal_visible(&fixture.overlay));
}

fn delete_without_trash_case(case: TrashlessDelete) {
    let held = Rc::new(HeldOperations::default());
    let fixture = held_fixture(
        &held,
        Rc::new(TrashlessLocal {
            read_only: case.read_only,
        }),
    );
    let browser = fixture.view.browser();
    assert_eq!(
        (browser.can_trash_at(0), browser.can_delete_at(0)),
        (Some(false), Some(!case.read_only)),
        "precondition"
    );
    if case.tenxer {
        enable_tenxer(&fixture);
    }
    press_delete_on_a(&fixture, case.key);
    pump(100);
    if case.read_only {
        read_only_keeps_the_trash_attempt(&fixture, &held);
        return;
    }
    without_trash_cancel_is_focused_and_enter_cancels(&fixture, &held, case);
    confirm_deletes_permanently(&fixture, &held, case.key);
}

#[test]
fn delete_without_trash_support_confirms_permanently_with_cancel_focused() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::trashless::delete_without_trash_support_confirms_permanently_with_cancel_focused",
        || {
            for (read_only, tenxer, key) in [
                (false, false, Key::Delete),
                (false, true, Key::d),
                (false, true, Key::Delete),
                (true, false, Key::Delete),
            ] {
                delete_without_trash_case(TrashlessDelete {
                    read_only,
                    tenxer,
                    key,
                });
            }
        },
    );
}

/// Pretends the fixture folder is an sshfs mount and returns its mount point.
fn mount_fixture_as_sshfs(directory: &Path) -> String {
    // The folder probe resolves symlinks before looking up the mount.
    let mount_point = std::fs::canonicalize(directory).expect("fixture");
    let mount_point = mount_point.to_str().expect("UTF-8 fixture path").to_owned();
    crate::adapters::set_mountinfo_for_isolated_test(Some(format!(
        "1 0 8:1 / / rw - ext4 /dev/root rw\n\
         900 1 0:90 / {mount_point} rw - fuse.sshfs remote:/share rw\n"
    )));
    mount_point
}

/// (no-Trash title, explanation, mount reason, provider reach)
fn explains_this_mount(fixture: &KeyboardFixture, mount_point: &str) -> (bool, bool, bool, bool) {
    (
        modal_text_contains(fixture, "Delete 1 item without Trash?"),
        modal_text_contains(fixture, "This location doesn't support Trash."),
        modal_text_contains(
            fixture,
            &format!("Trash isn't available on {mount_point} (fuse.sshfs)."),
        ),
        modal_text_contains(fixture, "depends on the storage provider"),
    )
}

/// The first Delete tries Trash; the adapter records the refusal before it
/// reports the failure.
fn first_delete_is_refused_by_the_mount(
    fixture: &KeyboardFixture,
    held: &HeldOperations,
    mount_point: &str,
) {
    let directory = fixture._directory.path();
    press_delete_on_a(fixture, Key::Delete);
    wait_until(|| !held.delete_requests.borrow().is_empty());
    let (id, permanent, _) = held.delete_requests.borrow()[0];
    assert!(!permanent, "the first Delete tries Trash");
    crate::adapters::trash_support::record_unsupported(
        crate::adapters::mount_watch::latest()
            .mount_for(directory)
            .expect("fixture mount"),
        crate::adapters::trash_support::UnsupportedReason::FuseRefused,
    );
    held.emit(
        id,
        crate::services::OperationEvent::CompletedWithErrors {
            request_id: id,
            deleted_locations: Vec::new(),
            retryable_locations: vec![Location::local(directory.join("a.txt"))],
            has_non_retryable_failures: false,
            message: "a.txt: This location doesn't support Trash.".into(),
        },
    );
    wait_focused_button(fixture, "Cancel");
    assert_eq!(
        explains_this_mount(fixture, mount_point),
        (true, true, true, true),
        "after the failed attempt: (no-Trash title, explanation, mount reason, provider reach)"
    );
    assert!(modal_key(fixture, Key::Return));
    wait_until(|| !modal_visible(&fixture.overlay));
    wait_until(|| fixture.view.browser().can_trash_at(0) == Some(false));
}

fn next_delete_confirms_up_front(
    fixture: &KeyboardFixture,
    held: &HeldOperations,
    mount_point: &str,
) {
    press_delete_on_a(fixture, Key::Delete);
    wait_focused_button(fixture, "Cancel");
    assert_eq!(
        (
            held.delete_requests.borrow().len(),
            explains_this_mount(fixture, mount_point)
        ),
        (1, (true, true, true, true)),
        "the next Delete confirms up front: (delete requests, explanation)"
    );
    assert!(modal_key(fixture, Key::Return));
    wait_until(|| !modal_visible(&fixture.overlay));
    assert!(
        fixture._directory.path().join("a.txt").exists(),
        "Enter cancels"
    );
}

fn reload_forgets_the_refusal(fixture: &KeyboardFixture, held: &HeldOperations) {
    let browser = fixture.view.browser();
    focus_files(fixture);
    plain(fixture, Key::F5);
    wait_until(|| browser.can_trash_at(0) == Some(true));
    assert!(
        crate::adapters::trash_support::unsupported_reason_for(fixture._directory.path()).is_none(),
        "Reload forgets the refusal"
    );
    move_to_named(fixture, &browser, "a.txt");
    plain(fixture, Key::Delete);
    wait_until(|| held.delete_requests.borrow().len() == 2);
    let (_, permanent, count) = held.delete_requests.borrow()[1];
    assert_eq!(
        (permanent, count, modal_visible(&fixture.overlay)),
        (false, 1, false),
        "Delete tries Trash again after Reload"
    );
}

/// GIO reports Trash on most FUSE roots; a refusal the adapter recorded
/// overrides that until the user reloads.
#[test]
fn a_mount_that_refused_trash_confirms_up_front_until_reload() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::trashless::a_mount_that_refused_trash_confirms_up_front_until_reload",
        || {
            let held = Rc::new(HeldOperations::default());
            let fixture = held_fixture(&held, Rc::new(crate::adapters::LocalFileSource));
            assert_eq!(
                fixture.view.browser().can_trash_at(0),
                Some(true),
                "precondition: home Trash"
            );
            let mount_point = mount_fixture_as_sshfs(fixture._directory.path());

            first_delete_is_refused_by_the_mount(&fixture, &held, &mount_point);
            next_delete_confirms_up_front(&fixture, &held, &mount_point);
            reload_forgets_the_refusal(&fixture, &held);
        },
    );
}
