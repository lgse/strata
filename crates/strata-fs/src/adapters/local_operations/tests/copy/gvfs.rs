// SPDX-License-Identifier: MIT

use super::*;

fn gvfs_bridge() -> Quirks {
    Quirks {
        mount_type: Some(("fuse.gvfsd-fuse", "rw,user_id=1000,group_id=1000")),
        ..Quirks::default()
    }
}

#[test]
fn names_moved_within_one_gvfs_bridge_backend_are_kept_and_fixed_between_two()
-> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let bridge = root.path().join("bridge");
    let server = bridge.join("sftp:host=example.org");
    let phone = bridge.join("mtp:host=Google_Pixel_8");
    for folder in [
        server.join("dir"),
        phone.join("Music"),
        phone.join("Ringtones"),
    ] {
        fs::create_dir_all(folder)?;
    }
    for name in ["dir/a:b", "c:d"] {
        fs::write(server.join(name), name)?;
    }
    fs::write(phone.join("Music/con.mp3"), b"song")?;
    let _bridge = FsQuirks::install(&bridge, gvfs_bridge());

    paste_into(&phone, &[server.join("dir"), server.join("c:d")]);
    run_paste(PasteRequest {
        id: OperationRequestId(1590),
        destination: Location::local(phone.join("Ringtones")),
        items: vec![PasteItem {
            source: Location::local(phone.join("Music/con.mp3")),
            conflict: TransferConflict::FailIfExists,
            target_name: None,
        }],
        move_sources: true,
    });

    assert_eq!(names(&phone)?, ["Music", "Ringtones", "c_d", "dir"]);
    assert_eq!(names(&phone.join("dir"))?, ["a_b"]);
    assert_eq!(names(&phone.join("Ringtones"))?, ["con.mp3"]);
    Ok(())
}

fn refuses_only_a_colon_b(name: &OsStr) -> Option<Errno> {
    (name == "a:b").then_some(Errno::INVAL)
}

#[test]
fn each_gvfs_bridge_backend_learns_only_from_its_own_refusals() -> Result<(), Box<dyn Error>> {
    let _serial = serial()?;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source");
    let bridge = root.path().join("bridge");
    let refusing = bridge.join("sftp:host=windows.example.org");
    let other = bridge.join("sftp:host=linux.example.org");
    for folder in ["one", "two", "links"] {
        fs::create_dir_all(source.join(folder))?;
    }
    for folder in [&refusing, &other] {
        fs::create_dir_all(folder)?;
    }
    for name in ["one/a:b", "two/c:d", "links/target"] {
        fs::write(source.join(name), name)?;
    }
    std::os::unix::fs::symlink("target", source.join("links/link"))?;
    let scope = FsQuirks::install(
        &bridge,
        Quirks {
            invalid_name: Some(refuses_only_a_colon_b),
            fail: vec![(Op::Symlink, "link", 0, Errno::NOSYS)],
            ..gvfs_bridge()
        },
    );

    paste_into(&refusing, &[source.join("one"), source.join("links")]);
    paste_into(&refusing, &[source.join("two")]);
    scope.clear_calls();
    paste_into(&other, &[source.join("two"), source.join("links")]);

    assert_eq!(names(&refusing.join("one"))?, ["a_b"]);
    assert_eq!(names(&refusing.join("links"))?, ["target"]);
    assert_eq!(names(&refusing.join("two"))?, ["c_d"]);
    assert_eq!(names(&other.join("two"))?, ["c:d"]);
    assert_eq!(names(&other.join("links"))?, ["link", "target"]);
    assert_eq!(scope.count(Op::Symlink), 1);
    Ok(())
}
