// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
#[ignore = "requires GVfs Trash and dbus-run-session; creates its own disposable HOME/XDG and session bus"]
fn isolated_trash_supports_read_copy_move_restore_and_delete() {
    const NAME: &str = "adapters::local_operations::tests::trash_capabilities::isolated_trash_supports_read_copy_move_restore_and_delete";
    const CHILD: &str = "STRATA_TRASH_CAPABILITIES_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let fixture = tempfile::tempdir().expect("isolated Trash");
        for name in ["home", "data", "config", "cache", "runtime"] {
            let path = fixture.path().join(name);
            fs::create_dir(&path).expect("fixture directory");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("private fixture");
        }
        let status = std::process::Command::new("dbus-run-session")
            .arg("--")
            .arg(std::env::current_exe().expect("test executable"))
            .args(["--exact", NAME, "--ignored", "--nocapture"])
            .env(CHILD, "1")
            .env("HOME", fixture.path().join("home"))
            .env("XDG_DATA_HOME", fixture.path().join("data"))
            .env("XDG_CONFIG_HOME", fixture.path().join("config"))
            .env("XDG_CACHE_HOME", fixture.path().join("cache"))
            .env("XDG_RUNTIME_DIR", fixture.path().join("runtime"))
            .env_remove("GIO_USE_VFS")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .status()
            .expect("isolated session starts");
        assert!(status.success());
        return;
    }

    let home = glib::home_dir();
    let folder = home.join("strata-433-folder");
    fs::create_dir(&folder).expect("source folder");
    fs::write(folder.join("notes.txt"), b"readable text").expect("source text");
    gio::File::for_path(&folder)
        .trash(gio::Cancellable::NONE)
        .expect("trash fixture folder");
    let trash = gio::File::for_uri("trash:///strata-433-folder");
    let child = trash.child("notes.txt");
    let attributes = "access::can-read,access::can-write,access::can-rename,access::can-delete";
    for (file, deletable) in [(&trash, true), (&child, false)] {
        let info = file
            .query_info(
                attributes,
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .expect("Trash capabilities");
        assert!(info.boolean("access::can-read"));
        assert!(!info.boolean("access::can-write"));
        assert!(!info.boolean("access::can-rename"));
        assert_eq!(info.boolean("access::can-delete"), deletable);
    }
    assert_eq!(
        child
            .load_contents(gio::Cancellable::NONE)
            .expect("read Trash")
            .0
            .as_ref(),
        b"readable text"
    );
    assert!(
        child
            .set_display_name("renamed.txt", gio::Cancellable::NONE)
            .is_err()
    );
    assert!(
        trash
            .child("new")
            .make_directory(gio::Cancellable::NONE)
            .is_err()
    );
    assert!(
        trash
            .child("new.txt")
            .create(gio::FileCreateFlags::NONE, gio::Cancellable::NONE)
            .is_err()
    );

    let context = glib::MainContext::default();

    use crate::services::{
        PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
    };
    let preview_events = Rc::new(RefCell::new(Vec::new()));
    let emitted = preview_events.clone();
    let provider = crate::adapters::LocalPreviewProvider::new(Rc::new(|| {
        crate::sandbox::MediaPreviewBackend::Software
    }));
    let _preview = provider.load(
        PreviewRequest {
            id: PreviewRequestId(433),
            entry: FileEntry {
                location: Location::uri(child.uri()),
                ..file_entry(&folder.join("notes.txt"))
            },
            text_byte_limit: 100,
            pdf_page: 0,
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while preview_events.borrow().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "text preview timed out"
        );
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        matches!(&preview_events.borrow()[0], PreviewEvent::Ready(preview)
        if matches!(&preview.content, PreviewContent::Text { content, .. } if content == "readable text")),
        "text preview/print input: {:?}",
        preview_events.borrow()
    );

    let copied = home.join("copied");
    context
        .block_on(copy_recursively(
            trash.clone(),
            gio::File::for_path(&copied),
            false,
            gio::Cancellable::new(),
            None,
        ))
        .expect("recursive copy out of Trash");
    assert_eq!(
        fs::read(copied.join("notes.txt")).expect("copied content"),
        b"readable text"
    );
    assert!(
        context
            .block_on(move_local(
                child.clone(),
                gio::File::for_path(home.join("child.txt")),
                gio::Cancellable::new(),
                None
            ))
            .is_err()
    );
    let moved = home.join("moved");
    context
        .block_on(move_local(
            trash.clone(),
            gio::File::for_path(&moved),
            gio::Cancellable::new(),
            None,
        ))
        .expect("move whole item out of Trash");
    assert_eq!(
        fs::read(moved.join("notes.txt")).expect("moved content"),
        b"readable text"
    );
    assert!(!trash.query_exists(gio::Cancellable::NONE));

    let restored = home.join("strata-433-restore.txt");
    fs::write(&restored, b"restore me").expect("restore source");
    gio::File::for_path(&restored)
        .trash(gio::Cancellable::NONE)
        .expect("trash restore fixture");
    let entry = FileEntry {
        location: Location::uri("trash:///strata-433-restore.txt"),
        ..file_entry(&restored)
    };
    let events = Rc::new(RefCell::new(Vec::new()));
    let emitted = events.clone();
    let _operation = LocalOperationProvider.restore(
        RestoreRequest {
            id: OperationRequestId(433),
            source: RestoreSource::TrashEntries(vec![entry]),
        },
        Rc::new(move |event| emitted.borrow_mut().push(event)),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !restored.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "restore failed: {:?}",
            events.borrow()
        );
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        fs::read(&restored).expect("restored content"),
        b"restore me"
    );
    let deleted = home.join("strata-433-delete.txt");
    fs::write(&deleted, b"delete me").expect("delete source");
    gio::File::for_path(&deleted)
        .trash(gio::Cancellable::NONE)
        .expect("trash delete fixture");
    let deleted = gio::File::for_uri("trash:///strata-433-delete.txt");
    deleted
        .delete(gio::Cancellable::NONE)
        .expect("permanently delete top-level item");
    assert!(!deleted.query_exists(gio::Cancellable::NONE));
}
