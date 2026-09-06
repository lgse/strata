// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    env,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{glib, prelude::*};

use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    services::{DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError},
    ui::{
        browser::{BrowserView, PeekBehavior, entry_icon},
        browser_modes::BrowserMode,
        thumbnail, thumbnail_cache,
    },
};

struct TrashFiles(Vec<FileEntry>);

impl FileSource for TrashFiles {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Batch {
            request_id: request.id,
            entries: self.0.clone(),
        });
        emit(DirectoryEvent::Finished {
            request_id: request.id,
            truncated: false,
            can_trash: Some(false),
            can_delete: Some(true),
        });
        LoadHandle::new(|| {})
    }
}

fn entry(name: &str, kind: EntryKind, path: &Path) -> FileEntry {
    FileEntry {
        location: Location::uri(format!("trash:///{name}")),
        thumbnail_path: Some(path.to_owned()),
        native_name: name.into(),
        display_name: name.into(),
        kind,
        size: MetadataValue::Known(42),
        modified_unix_seconds: MetadataValue::Known(1),
        mode: MetadataValue::Unavailable,
        is_hidden: false,
    }
}

#[test]
fn trash_thumbnails_and_fallback_icons_work_in_every_view() {
    const CHILD: &str = "STRATA_TRASH_THUMBNAIL_TEST_CHILD";
    if env::var_os(CHILD).is_none() {
        let sandbox = tempfile::tempdir().expect("isolated preferences and thumbnail cache");
        let status = std::process::Command::new(env::current_exe().expect("test executable"))
            .args(["--exact", "ui::thumbnail::tests::trash::trash_thumbnails_and_fallback_icons_work_in_every_view", "--nocapture"])
            .env(CHILD, "1")
            .env("XDG_CONFIG_HOME", sandbox.path().join("config"))
            .env("XDG_CACHE_HOME", sandbox.path().join("cache"))
            .env("XDG_DATA_HOME", sandbox.path().join("data"))
            .status().expect("isolated GTK test");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }
    crate::assets::prepare().expect("assets");
    crate::assets::register_icon_theme();
    crate::ui::theme::ThemeManager::shared();
    crate::ui::window::load_styles();
    let fixture = tempfile::tempdir().expect("fixture");
    let path = fixture.path().join("photo.png.2");
    let pixbuf = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 64, 48)
        .expect("image fixture");
    pixbuf.fill(0x6699ccff);
    pixbuf.new_subpixbuf(0, 24, 64, 24).fill(0x448844ff);
    let png = pixbuf.save_to_bufferv("png", &[]).expect("PNG fixture");
    std::fs::write(&path, &png).expect("image file");
    thumbnail_cache::store(&path, 1, &png);
    assert!(thumbnail_cache::lookup(&path, 1).is_some());
    let image = entry("photo.png", EntryKind::File, &path);
    let directory = entry(
        "folder.png",
        EntryKind::Directory,
        &fixture.path().join("folder.png"),
    );
    let code = entry(
        "script.rs",
        EntryKind::File,
        &fixture.path().join("script.rs"),
    );
    assert_eq!(entry_icon(&image), crate::assets::icons::PICTURES);
    assert_eq!(entry_icon(&directory), crate::assets::icons::FOLDER);
    assert_eq!(entry_icon(&code), crate::assets::icons::FILE_CODE);
    let view = BrowserView::new(
        Rc::new(TrashFiles(vec![image, directory, code])),
        PeekBehavior::default(),
    );
    let browser = view.browser();
    let window = gtk::Window::builder()
        .child(&view.widget())
        .default_width(1000)
        .default_height(600)
        .build();
    window.present();
    browser.navigate(Location::uri("trash:///"));
    for mode in [BrowserMode::Icons, BrowserMode::Columns, BrowserMode::List] {
        view.set_view_mode(mode);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !has_visible_thumbnail(&path) {
            assert!(
                Instant::now() < deadline,
                "Trash thumbnail not displayed: {mode:?}"
            );
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            browser.column_snapshot(0).expect("Trash column").location,
            Location::uri("trash:///")
        );
    }
    thumbnail::cancel_thumbnails_in(&view.widget());
    browser.clear_observer();
    window.destroy();
}

#[test]
fn metadata_updates_use_the_local_trash_thumbnail_source() {
    let path = Path::new("/fixture/Trash/files/photo.png.2");
    let entry = entry("photo.png", EntryKind::File, path);
    thumbnail::SETTLE_VIEWS.with(|views| {
        views.borrow_mut().insert(
            0,
            thumbnail::ViewSettle {
                viewport: glib::WeakRef::new(),
                pending: vec![thumbnail::SettledPark {
                    key: thumbnail::ThumbnailKey {
                        path: path.to_owned(),
                        modified: None,
                        file_size: None,
                        thumbnail_size: 64,
                    },
                    kind: thumbnail::ThumbnailKind::Image,
                    target: thumbnail::PendingTarget {
                        image_id: 1,
                        request: 1,
                        image: glib::WeakRef::new(),
                    },
                    wait_for_metadata: true,
                }],
                timer: None,
                first_park: None,
                hooked: false,
            },
        );
    });
    thumbnail::note_metadata_entry(&entry);
    thumbnail::SETTLE_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        let pending = &views[&0].pending[0];
        assert_eq!(pending.key.modified, Some(1));
        assert_eq!(pending.key.file_size, Some(42));
        assert!(!pending.wait_for_metadata);
        views.clear();
    });
}

fn has_visible_thumbnail(path: &Path) -> bool {
    thumbnail::TRACKED_THUMBNAILS.with(|tracked| {
        tracked.borrow().iter().any(|tracked| {
            tracked.path == path
                && tracked
                    .image
                    .upgrade()
                    .is_some_and(|image| image.is_mapped())
        })
    })
}
