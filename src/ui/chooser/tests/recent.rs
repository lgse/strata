// SPDX-License-Identifier: MIT

use super::acceptance::{request, wait_until};
use super::*;

struct RecentSource(Vec<FileEntry>);

impl FileSource for RecentSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        if !request.location.is_recent_root() {
            return LocalFileSource.enumerate(request, emit);
        }
        let entries = self.0.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries,
            });
            emit(DirectoryEvent::Finished {
                request_id: request.id,
                truncated: false,
                can_trash: Some(false),
                can_delete: Some(false),
            });
        });
        LoadHandle::new(move || task.abort())
    }
}

#[test]
fn recent_chooser_filters_remote_targets_and_returns_local_open_and_save_paths() {
    crate::test_support::gtk_test(
        "ui::chooser::tests::recent::recent_chooser_filters_remote_targets_and_returns_local_open_and_save_paths",
        || {
            crate::ui::prepare_portal_ui();
            PreferenceManager::shared().set_browser_mode(BrowserMode::List);
            let root = tempfile::tempdir().expect("fixture");
            let target = root.path().join("existing.txt");
            std::fs::write(&target, "existing").expect("file");
            let mut local = entry("existing.txt", EntryKind::File);
            local.location = Location::local(&target);
            let mut remote = entry("remote.txt", EntryKind::File);
            remote.location = Location::uri("smb://server/share/remote.txt");
            let mut filtered = entry("image.png", EntryKind::File);
            filtered.location = Location::local(root.path().join("image.png"));
            for save in [false, true] {
                let mut request = request(root.path().to_path_buf());
                request.filters = vec![FileFilter::new("Text").glob("*.txt")];
                if save {
                    request.kind = ChooserKind::SaveFile {
                        current_name: Some("suggested.txt".into()),
                    };
                }
                let result = Rc::new(RefCell::new(None));
                let received = result.clone();
                let source = Rc::new(ChooserFileSource {
                    source: Rc::new(RecentSource(vec![
                        local.clone(),
                        remote.clone(),
                        filtered.clone(),
                    ])),
                    filter: Rc::new(RefCell::new(None)),
                    directory_only: Rc::new(Cell::new(false)),
                });
                let state = build_chooser_with_source(
                    request,
                    Arc::new(AtomicBool::new(false)),
                    move |value| {
                        received.replace(Some(value));
                    },
                    source,
                )
                .expect("chooser");
                let browser = state.view.browser();
                browser.navigate(Location::uri("recent:///"));
                wait_until(|| {
                    browser.column_snapshot(0).is_some_and(|column| {
                        column.location.is_recent_root() && !column.loading && column.count == 1
                    })
                });
                assert_eq!(
                    browser.entry_at(0, 0).expect("local target").location,
                    Location::local(&target)
                );
                if save {
                    assert_eq!(
                        state.filename.as_ref().expect("filename").text(),
                        "suggested.txt"
                    );
                    state.accept_button.emit_clicked();
                    assert!(result.borrow().is_none());
                    assert!(state.error.is_visible());
                }
                browser.select(0, 0);
                let expected = if save {
                    let filename = state.filename.as_ref().expect("filename");
                    assert_eq!(filename.text(), "existing.txt");
                    filename.set_text("new.txt");
                    root.path().join("new.txt")
                } else {
                    target.clone()
                };
                state.accept_button.emit_clicked();
                wait_until(|| result.borrow().is_some());
                let selected = result
                    .borrow_mut()
                    .take()
                    .expect("result")
                    .expect("accepted");
                assert_eq!(selected.uris().len(), 1);
                assert_eq!(
                    selected.uris()[0].to_string(),
                    gio::File::for_path(expected).uri()
                );
            }
        },
    );
}
