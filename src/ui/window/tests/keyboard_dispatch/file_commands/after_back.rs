// SPDX-License-Identifier: MIT

use std::cell::RefCell;

use super::*;

type DirectoryEmit = Rc<dyn Fn(crate::services::DirectoryEvent)>;

/// Local browsing that holds back the listing of one folder until `release`.
#[derive(Default)]
struct HeldListingLocal {
    held: RefCell<Option<Location>>,
    pending: RefCell<Vec<(crate::services::DirectoryRequest, DirectoryEmit)>>,
    loads: RefCell<Vec<crate::services::LoadHandle>>,
}

impl HeldListingLocal {
    fn release(&self) {
        self.held.take();
        let pending = self.pending.take();
        for (request, emit) in pending {
            let load = crate::services::FileSource::enumerate(
                &crate::adapters::LocalFileSource,
                request,
                emit,
            );
            self.loads.borrow_mut().push(load);
        }
    }
}

impl crate::services::FileSource for HeldListingLocal {
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
        emit: DirectoryEmit,
    ) -> crate::services::LoadHandle {
        if self.held.borrow().as_ref() == Some(&request.location) {
            self.pending.borrow_mut().push((request, emit));
            return crate::services::LoadHandle::new(|| {});
        }
        crate::adapters::LocalFileSource.enumerate(request, emit)
    }

    fn supports_metadata_fill(&self, location: &Location) -> bool {
        crate::adapters::LocalFileSource.supports_metadata_fill(location)
    }

    fn fill_metadata(
        &self,
        request: crate::services::MetadataRequest,
        emit: DirectoryEmit,
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

/// Opens `alpha/sub/inner` as columns, then leaves for `alpha/a0` so Back
/// restores all four columns. Returns `alpha`.
fn open_alpha_tree_then_leave(fixture: &KeyboardFixture) -> PathBuf {
    let directory = fixture._directory.path().to_path_buf();
    let alpha = directory.join("alpha");
    std::fs::create_dir_all(alpha.join("a0")).expect("fixture folder");
    std::fs::create_dir_all(alpha.join("sub/inner")).expect("fixture folders");
    std::fs::write(alpha.join("sub/inner/x.txt"), b"x").expect("fixture file");
    let browser = fixture.view.browser();
    browser.refresh_all();
    wait_until(|| rendered_name(&fixture.view.widget(), "alpha"));
    for (depth, folder) in ["alpha", "alpha/sub", "alpha/sub/inner"]
        .into_iter()
        .enumerate()
    {
        browser.descend(depth, Location::local(directory.join(folder)));
        wait_loaded(&browser, depth + 1);
    }
    browser.navigate(Location::local(alpha.join("a0")));
    wait_loaded(&browser, 0);
    alpha
}

/// Back with `alpha` held, then Left twice from the restored `x.txt`.
fn left_twice_while_alpha_lists(fixture: &KeyboardFixture, source: &HeldListingLocal) {
    let browser = fixture.view.browser();
    browser.back();
    wait_loaded(&browser, 3);
    focus_files(fixture);
    wait_until(|| focused_widget_shows(&fixture.window, "x.txt"));
    plain(fixture, Key::Left);
    wait_until(|| focused_widget_shows(&fixture.window, "inner"));
    plain(fixture, Key::Left);
    assert_eq!(browser.active_depth(), Some(1));
    assert!(
        browser
            .column_snapshot(1)
            .is_some_and(|column| column.loading),
        "Left must reach 'alpha' before it lists"
    );
    source.release();
    wait_loaded(&browser, 1);
}

#[test]
fn left_into_a_parent_still_loading_after_back_focuses_its_open_folder() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::file_commands::after_back::left_into_a_parent_still_loading_after_back_focuses_its_open_folder",
        || {
            let source = Rc::new(HeldListingLocal::default());
            let view_source = source.clone();
            let fixture = KeyboardFixture::with_parts(Rc::new(TextPreview), move || {
                BrowserView::new(view_source, crate::ui::browser::PeekBehavior::default())
            });
            let alpha = open_alpha_tree_then_leave(&fixture);
            source.held.replace(Some(Location::local(&alpha)));
            left_twice_while_alpha_lists(&fixture, &source);

            let browser = fixture.view.browser();
            wait_until_or(
                || {
                    focused_widget_shows(&fixture.window, "sub")
                        && !focused_widget_shows(&fixture.window, "a0")
                },
                || {
                    format!(
                        "keyboard focus on the 'sub' row, cursor {:?}",
                        browser.focused_entry().map(|entry| entry.display_name)
                    )
                },
            );
            assert_eq!(
                browser
                    .rename_item()
                    .map(|(_, _, entry)| entry.display_name),
                Some("sub".into())
            );
        },
    );
}
