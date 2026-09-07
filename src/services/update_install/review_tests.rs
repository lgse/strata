// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn staged_elf_can_execute_and_replace_the_install() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let staged =
        stage_verified_binary(Path::new("/bin/true"), directory.path()).expect("stage ELF");
    let installed = directory.path().join("strata");
    staged.persist(&installed).expect("replace executable");
    confirm_replacement(&installed).expect("replacement runs");
}

#[test]
fn rollback_does_not_follow_a_preexisting_symlink() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let installed = directory.path().join("strata");
    let unrelated = directory.path().join("unrelated");
    fs::write(&installed, b"previous").expect("installed fixture");
    fs::write(&unrelated, b"untouched").expect("unrelated fixture");
    std::os::unix::fs::symlink(&unrelated, rollback_path(directory.path()))
        .expect("rollback symlink");
    let rollback = stage_rollback(&installed, directory.path()).expect("preserve executable");
    assert_eq!(
        fs::read(&unrelated).expect("unrelated contents"),
        b"untouched"
    );
    assert_eq!(fs::read(rollback).expect("rollback contents"), b"previous");
}

#[test]
fn rollback_replaces_the_inode_without_truncating_other_links() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let installed = directory.path().join("strata");
    let other_link = directory.path().join("other");
    fs::write(&installed, b"previous").expect("installed fixture");
    let rollback = stage_rollback(&installed, directory.path()).expect("preserve executable");
    fs::write(&installed, b"replacement").expect("replacement fixture");
    fs::hard_link(&installed, &other_link).expect("hard link");
    restore_rollback(&rollback, &installed).expect("restore executable");
    assert_eq!(
        fs::read(installed).expect("installed contents"),
        b"previous"
    );
    assert_eq!(
        fs::read(other_link).expect("linked contents"),
        b"replacement"
    );
}

#[test]
fn download_rejects_self_consistent_but_wrong_assets_and_dot_segments() {
    for (tag, asset) in [
        ("v0.11.2", "other.tar.gz"),
        ("..", "archive.tar.gz"),
        (".", "archive.tar.gz"),
    ] {
        assert!(
            verified_download_url(&InstallRequest {
                tag: tag.to_owned(),
                asset_name: asset.to_owned(),
                advertised_url: format!("{RELEASE_DOWNLOAD_ROOT}/{tag}/{asset}"),
            })
            .is_err()
        );
    }
}

#[test]
fn authenticated_package_must_match_the_selected_asset() {
    let request = InstallRequest {
        tag: "v0.11.2".to_owned(),
        asset_name: super::super::update_check::archive_name("0.11.2"),
        advertised_url: String::new(),
    };
    let package = Path::new(request.asset_name.trim_end_matches(".tar.gz"));
    assert!(verify_package_name(package, &request).is_ok());
    assert!(verify_package_name(Path::new("strata-other-version"), &request).is_err());
}

#[test]
fn metadata_rejects_both_advertised_and_streamed_overflow() {
    for limit in [manifest::MAX_MANIFEST_BYTES, manifest::MAX_SIGNATURES_BYTES] {
        for advertised in [false, true] {
            let mut builder = ureq::http::Response::builder();
            if advertised {
                builder = builder.header("content-length", limit + 1);
            }
            let data = if advertised {
                Vec::new()
            } else {
                vec![b'a'; limit as usize + 1]
            };
            let mut response = builder
                .body(ureq::Body::builder().data(data))
                .expect("metadata response");
            assert!(read_metadata_body(&mut response, limit, &InstallCancel::new()).is_err());
        }
    }
}

#[test]
fn metadata_cancellation_does_not_report_a_signature_error() {
    let mut response = ureq::http::Response::new(ureq::Body::builder().data(b"metadata".to_vec()));
    let cancel = InstallCancel::new();
    cancel.cancel();
    assert!(matches!(
        read_metadata_body(&mut response, 32, &cancel),
        Err(InstallStop::Cancelled)
    ));
}
