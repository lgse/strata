// SPDX-License-Identifier: MIT

use super::*;

#[derive(Clone, Default)]
struct FakeRemovalBackend {
    calls: Rc<RefCell<Vec<String>>>,
    outcomes: Rc<RefCell<HashMap<String, Result<(), String>>>>,
    hold: Rc<Cell<bool>>,
}

impl FakeRemovalBackend {
    fn fail(&self, uri: &str, error: &str) {
        self.outcomes
            .borrow_mut()
            .insert(uri.to_owned(), Err(error.to_owned()));
    }
}

impl RecentRemovalBackend for FakeRemovalBackend {
    fn delete(&self, uri: &str) -> RecentEnumerationFuture<Result<(), String>> {
        self.calls.borrow_mut().push(uri.to_owned());
        let hold = self.hold.clone();
        let outcome = self.outcomes.borrow().get(uri).cloned().unwrap_or(Ok(()));
        Box::pin(async move {
            while hold.get() {
                glib::timeout_future(Duration::from_millis(1)).await;
            }
            outcome
        })
    }
}

/// Recent removals against a fake backend, holding the main-context lock for the test.
struct Removals {
    state: RecentRemovalState,
    backend: Rc<FakeRemovalBackend>,
    _serial: crate::test_support::MainContextGuard<'static>,
}

impl Removals {
    fn new() -> Self {
        Self {
            _serial: serial(),
            state: RecentRemovalState::new(),
            backend: Rc::default(),
        }
    }

    fn remove(&self, uris: impl IntoIterator<Item = String>) {
        let backend: Rc<dyn RecentRemovalBackend> = self.backend.clone();
        recent_remove_entries_with_backend(&self.state, &backend, uris);
    }

    fn settle(&self) {
        drain_until(|| self.state.in_flight.borrow().is_empty());
    }

    fn remove_logged(&self, uri: &str) -> String {
        capture_logs(|| {
            self.remove([uri.to_owned()]);
            self.settle();
        })
    }

    fn calls(&self) -> Vec<String> {
        self.backend.calls.borrow().clone()
    }

    fn pause_for_a_moment() {
        glib::MainContext::default().block_on(glib::timeout_future(Duration::from_millis(10)));
    }
}

#[test]
fn recent_removal_calls_delete_and_logs_success() {
    let removals = Removals::new();
    let uri = "recent:///success";

    let output = removals.remove_logged(uri);

    assert_eq!(removals.calls(), [uri]);
    assert!(removals.state.in_flight.borrow().is_empty());
    assert_contains_all(&output, &["removed recent entry", uri]);
}

#[test]
fn recent_removal_failure_logs_a_warning_and_allows_retry() {
    let removals = Removals::new();
    let uri = "recent:///locked";
    removals.backend.fail(uri, "xbel is locked");

    let output = removals.remove_logged(uri);

    assert_eq!(removals.calls(), [uri]);
    assert_contains_all(
        &output,
        &["failed to remove recent entry", uri, "xbel is locked"],
    );
    removals.backend.outcomes.borrow_mut().remove(uri);
    removals.remove([uri.to_owned()]);
    removals.settle();
    assert_eq!(removals.calls(), [uri, uri]);
}

#[test]
fn recent_removal_logs_keep_credentials_out_of_the_default_log() {
    let removals = Removals::new();
    let uri = "recent://alice:s3cret;domain=CORP@host.example/item?token=abc#frag";
    removals.backend.fail(
        uri,
        "cannot reach sftp://alice:s3cret@host.example/item?token=abc#frag",
    );

    let output = removals.remove_logged(uri);

    let warning = crate::test_support::captured_event(&output, "failed to remove recent entry");
    assert_eq!(warning.split_whitespace().next(), Some("WARN"));
    assert!(warning.contains("recent://host.example/item"), "{warning}");
    assert_contains_none(&output, &["alice", "s3cret", "CORP", "token", "frag"]);
}

#[test]
fn rapid_double_invocation_on_the_same_uri_does_not_issue_two_concurrent_deletes() {
    let removals = Removals::new();
    removals.backend.hold.set(true);
    let uri = "recent:///double-click".to_owned();

    removals.remove([uri.clone()]);
    drain_until(|| !removals.backend.calls.borrow().is_empty());
    assert_eq!(
        removals.calls().len(),
        1,
        "the first removal should have called delete() once"
    );

    removals.remove([uri.clone()]);
    Removals::pause_for_a_moment();

    assert_eq!(
        removals.calls().len(),
        1,
        "a second removal for an in-flight URI must not call delete() again"
    );
    assert!(removals.state.in_flight.borrow().contains(&uri));
    removals.backend.hold.set(false);
    removals.settle();
}

#[test]
fn multi_selection_removal_issues_one_delete_per_uri() {
    let removals = Removals::new();
    let uris = vec![
        "recent:///one".to_owned(),
        "recent:///two".to_owned(),
        "recent:///three".to_owned(),
    ];

    removals.remove(uris.clone());
    removals.settle();

    let mut called = removals.calls();
    called.sort();
    let mut expected = uris;
    expected.sort();
    assert_eq!(called, expected);
}

#[test]
fn recent_removal_rejects_real_file_uris() {
    let removals = Removals::new();
    let fixture = tempfile::NamedTempFile::new().expect("target file");
    removals.remove([
        gio::File::for_path(fixture.path()).uri().to_string(),
        "smb://server/share/file".to_owned(),
    ]);
    Removals::pause_for_a_moment();
    assert!(removals.calls().is_empty());
    assert!(fixture.path().exists());
    assert!(removals.state.in_flight.borrow().is_empty());
}
