// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    APPLICATION_ICON, DESKTOP_ENTRY, InstallCancel, InstallRequest, InstallStop, UpdateMethod,
    aur_repository_version_from_response, desktop_entry_with_exec, download_to_file_bounded,
    package_repository_version_for, parse_aur_package_version, parse_package_version,
    refresh_desktop_metadata, repository_database_version, restore_rollback, stage_binary_path,
    stage_rollback, stage_workdir, update_method_for, verified_download_url, verify_staged_binary,
};

const PACKAGED_ENTRY: &str =
    "[Desktop Entry]\nType=Application\nName=Strata\nExec=strata %U\nIcon=io.github.lgse.Strata\n";

fn scratch_dir(label: &str, line: u32) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "strata-update-install-test-{label}-{}-{line}",
        std::process::id()
    ));
    let _removed = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn packaged_metadata(package_dir: &Path) {
    fs::create_dir_all(package_dir).expect("create package dir");
    fs::write(package_dir.join(DESKTOP_ENTRY), PACKAGED_ENTRY).expect("write packaged entry");
    fs::write(package_dir.join(APPLICATION_ICON), b"<svg/>").expect("write packaged icon");
}

fn installed_icon(data_home: &Path) -> PathBuf {
    data_home
        .join("icons/hicolor/scalable/apps")
        .join(APPLICATION_ICON)
}

fn ownership_probe(dir: &Path, exit_code: u8) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let probe = dir.join("pacman");
    fs::write(&probe, format!("#!/bin/sh\nexit {exit_code}\n")).expect("write ownership probe");
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755))
        .expect("make ownership probe executable");
    probe
}

#[test]
fn arch_package_versions_drop_epoch_and_package_release() {
    assert_eq!(
        parse_package_version("0.8.1-1").map(|version| version.to_string()),
        Some("0.8.1".to_owned())
    );
    assert_eq!(
        parse_package_version("2:0.9.0-rc.1-3.1").map(|version| version.to_string()),
        Some("0.9.0-rc.1".to_owned())
    );
    assert!(parse_package_version("0.8.1").is_none());
    assert!(parse_package_version("not-a-version-1").is_none());
}

#[test]
fn aur_package_versions_restore_upstream_prerelease_separators() {
    for (package, release) in [
        ("0.9.0-1", "0.9.0"),
        ("0.10.0rc.2-1", "0.10.0-rc.2"),
        ("2:0.10.0beta.1-3", "0.10.0-beta.1"),
    ] {
        assert_eq!(
            parse_aur_package_version(package).map(|version| version.to_string()),
            Some(release.to_owned())
        );
    }
}

#[test]
fn aur_response_selects_the_named_package() {
    let response = r#"{
        "resultcount": 2,
        "results": [
            {"Name": "another-package", "Version": "9.0.0-1"},
            {"Name": "strata-rc-bin", "Version": "0.10.0rc.2-1"}
        ]
    }"#;

    assert_eq!(
        aur_repository_version_from_response(response, "strata-rc-bin")
            .map(|version| version.to_string()),
        Ok("0.10.0-rc.2".to_owned())
    );
    assert!(aur_repository_version_from_response(response, "strata-bin").is_err());
}

#[test]
fn repository_probe_selects_strata_instead_of_a_dependency() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("create tempdir");
    let probe = dir.path().join("pacman");
    fs::write(
        &probe,
        "#!/bin/sh\nprintf '%s\\n' 'dependency 9.8.7-1' 'strata 0.8.1-1'\n",
    )
    .expect("write probe");
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).expect("make probe executable");

    assert_eq!(
        package_repository_version_for(&probe, "strata").map(|version| version.to_string()),
        Ok("0.8.1".to_owned())
    );
}

#[test]
fn omarchy_database_reports_the_packaged_strata_version() {
    let description = b"%NAME%\nstrata\n\n%VERSION%\n0.8.1-2\n";
    let mut archive = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut archive);
        let mut header = tar::Header::new_gnu();
        header.set_size(description.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "strata-0.8.1-2/desc", &description[..])
            .expect("append package description");
        builder.finish().expect("finish repository archive");
    }
    let database = zstd::stream::encode_all(&archive[..], 0).expect("compress repository database");

    assert_eq!(
        repository_database_version(&database, "strata").map(|version| version.to_string()),
        Ok("0.8.1".to_owned())
    );
}

#[test]
fn omarchy_package_ownership_defers_updates_to_omarchy() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let probe = ownership_probe(dir.path(), 0);
    let os_release = dir.path().join("os-release");
    fs::write(&os_release, "NAME=\"Omarchy\"\nID=omarchy\nID_LIKE=arch\n")
        .expect("write os-release");

    assert_eq!(
        update_method_for(Path::new("/usr/bin/strata"), &probe, &os_release),
        UpdateMethod::Omarchy
    );
}

#[test]
fn ownership_probe_errors_disable_in_place_updates() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let os_release = dir.path().join("os-release");
    fs::write(&os_release, "ID=omarchy\n").expect("write os-release");

    assert_eq!(
        update_method_for(Path::new("/usr/bin/strata"), dir.path(), &os_release),
        UpdateMethod::Omarchy
    );
}

#[test]
fn non_omarchy_pacman_ownership_defers_updates_to_pacman() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let probe = ownership_probe(dir.path(), 0);
    let os_release = dir.path().join("os-release");
    fs::write(&os_release, "ID=arch\n").expect("write os-release");

    assert_eq!(
        update_method_for(Path::new("/usr/bin/strata"), &probe, &os_release),
        UpdateMethod::Pacman
    );
}

#[test]
fn unowned_release_binary_keeps_in_place_updates() {
    let dir = tempfile::tempdir().expect("create scratch dir");
    let probe = ownership_probe(dir.path(), 1);

    assert_eq!(
        update_method_for(
            Path::new("/home/user/.local/bin/strata"),
            &probe,
            &dir.path().join("missing-os-release"),
        ),
        UpdateMethod::InPlace
    );
}

#[test]
fn stage_workdir_is_unique_per_call() {
    let exe_dir = tempfile::tempdir().expect("create scratch exe dir");
    let first = stage_workdir(exe_dir.path()).expect("stage first workdir");
    let second = stage_workdir(exe_dir.path()).expect("stage second workdir");

    assert_ne!(first.path(), second.path());
    assert!(first.path().is_dir());
    assert!(second.path().is_dir());
    // Both live inside `exe_dir`, matching the old process-scoped scheme's
    // placement, just no longer sharing a single path within it.
    assert_eq!(first.path().parent(), Some(exe_dir.path()));
    assert_eq!(second.path().parent(), Some(exe_dir.path()));
}

#[test]
fn stage_binary_path_is_unique_per_call() {
    let exe_dir = tempfile::tempdir().expect("create scratch exe dir");
    let first = stage_binary_path(exe_dir.path()).expect("stage first binary path");
    let second = stage_binary_path(exe_dir.path()).expect("stage second binary path");

    assert_ne!(first.path(), second.path());
    assert_eq!(first.path().parent(), Some(exe_dir.path()));
    assert_eq!(second.path().parent(), Some(exe_dir.path()));
}

#[test]
fn desktop_entry_exec_points_at_the_install_path_and_keeps_field_codes() {
    let entry = desktop_entry_with_exec(PACKAGED_ENTRY, Path::new("/home/user/.local/bin/strata"));

    assert!(entry.contains("Exec=/home/user/.local/bin/strata %U\n"));
    assert!(entry.starts_with("[Desktop Entry]\n"));
    assert!(entry.contains("Icon=io.github.lgse.Strata\n"));
}

#[test]
fn desktop_entry_exec_quotes_paths_containing_spaces() {
    let entry = desktop_entry_with_exec(PACKAGED_ENTRY, Path::new("/opt/my apps/strata"));

    assert!(entry.contains("Exec=\"/opt/my apps/strata\" %U\n"));
}

#[test]
fn desktop_entry_without_field_codes_keeps_a_bare_exec() {
    let entry = desktop_entry_with_exec(
        "[Desktop Entry]\nExec=strata\n",
        Path::new("/usr/bin/strata"),
    );

    assert_eq!(entry, "[Desktop Entry]\nExec=/usr/bin/strata\n");
}

#[test]
fn refresh_rewrites_an_installed_entry_and_icon() {
    let dir = scratch_dir("refresh", line!());
    let package_dir = dir.join("strata-0.7.0-x86_64-unknown-linux-gnu");
    packaged_metadata(&package_dir);
    let data_home = dir.join("share");
    let applications = data_home.join("applications");
    fs::create_dir_all(&applications).expect("create applications dir");
    fs::write(
        applications.join(DESKTOP_ENTRY),
        "[Desktop Entry]\nExec=strata %U\nIcon=system-file-manager\n",
    )
    .expect("write stale entry");
    let executable = dir.join("bin/strata");

    refresh_desktop_metadata(&package_dir, &executable, &data_home);

    let entry = fs::read_to_string(applications.join(DESKTOP_ENTRY)).expect("read entry");
    assert!(entry.contains(&format!("Exec={} %U\n", executable.display())));
    assert!(entry.contains("Icon=io.github.lgse.Strata\n"));
    assert_eq!(
        fs::read(installed_icon(&data_home)).expect("read icon"),
        b"<svg/>"
    );

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn refresh_does_not_create_metadata_the_user_never_installed() {
    let dir = scratch_dir("no-entry", line!());
    let package_dir = dir.join("strata-0.7.0-x86_64-unknown-linux-gnu");
    packaged_metadata(&package_dir);
    let data_home = dir.join("share");

    refresh_desktop_metadata(&package_dir, &dir.join("bin/strata"), &data_home);

    assert!(!data_home.join("applications").join(DESKTOP_ENTRY).exists());
    assert!(!installed_icon(&data_home).exists());

    fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn refresh_keeps_an_installed_entry_when_the_archive_omits_metadata() {
    let dir = scratch_dir("legacy-archive", line!());
    let package_dir = dir.join("strata-0.7.0-x86_64-unknown-linux-gnu");
    fs::create_dir_all(&package_dir).expect("create package dir");
    let data_home = dir.join("share");
    let applications = data_home.join("applications");
    fs::create_dir_all(&applications).expect("create applications dir");
    let existing = "[Desktop Entry]\nExec=strata %U\n";
    fs::write(applications.join(DESKTOP_ENTRY), existing).expect("write entry");

    refresh_desktop_metadata(&package_dir, &dir.join("bin/strata"), &data_home);

    assert_eq!(
        fs::read_to_string(applications.join(DESKTOP_ENTRY)).expect("read entry"),
        existing
    );

    fs::remove_dir_all(&dir).expect("cleanup");
}

fn request(tag: &str, asset: &str, advertised: &str) -> InstallRequest {
    InstallRequest {
        tag: tag.to_owned(),
        asset_name: asset.to_owned(),
        advertised_url: advertised.to_owned(),
    }
}

const TAG: &str = "v0.11.2";
const ASSET: &str = "strata-0.11.2-x86_64-unknown-linux-gnu.tar.gz";

fn release_url(tag: &str, asset: &str) -> String {
    format!("https://github.com/lgse/strata/releases/download/{tag}/{asset}")
}

#[test]
fn download_url_is_derived_from_the_release_tag_and_asset() {
    let asset = super::super::update_check::archive_name("0.11.2");
    let expected = release_url(TAG, &asset);

    let url = verified_download_url(&request(TAG, &asset, &expected)).expect("url should verify");

    assert_eq!(url, expected);
}

#[test]
fn download_url_rejects_another_host() {
    let advertised = format!("https://example.invalid/lgse/strata/releases/download/{TAG}/{ASSET}");

    assert!(verified_download_url(&request(TAG, ASSET, &advertised)).is_err());
}

#[test]
fn download_url_rejects_another_repository() {
    let advertised = format!("https://github.com/attacker/strata/releases/download/{TAG}/{ASSET}");

    assert!(verified_download_url(&request(TAG, ASSET, &advertised)).is_err());
}

#[test]
fn download_url_rejects_a_plaintext_scheme() {
    let advertised = release_url(TAG, ASSET).replace("https://", "http://");

    assert!(verified_download_url(&request(TAG, ASSET, &advertised)).is_err());
}

#[test]
fn download_url_rejects_an_asset_from_another_tag() {
    let advertised = release_url("v0.11.1", ASSET);

    assert!(verified_download_url(&request(TAG, ASSET, &advertised)).is_err());
}

#[test]
fn download_url_rejects_an_unexpected_asset_name() {
    let advertised = release_url(TAG, "strata-0.11.2-x86_64-unknown-linux-gnu.debug");

    assert!(verified_download_url(&request(TAG, ASSET, &advertised)).is_err());
}

#[test]
fn download_url_rejects_embedded_credentials() {
    let advertised = release_url(TAG, ASSET).replace("https://", "https://user:token@");

    assert!(verified_download_url(&request(TAG, ASSET, &advertised)).is_err());
}

#[test]
fn download_url_rejects_a_tag_that_escapes_the_release_path() {
    let tag = "v0.11.2/../../../attacker/strata/releases/download/v1";
    let advertised = release_url(tag, ASSET);

    assert!(verified_download_url(&request(tag, ASSET, &advertised)).is_err());
}

#[test]
fn download_url_rejects_an_asset_name_containing_a_path_segment() {
    let asset = "../../../attacker.tar.gz";
    let advertised = release_url(TAG, asset);

    assert!(verified_download_url(&request(TAG, asset, &advertised)).is_err());
}

struct StubServer {
    url: String,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl StubServer {
    fn serving(headers: String, body: Vec<u8>) -> Self {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback stub server");
        let url = format!(
            "http://{}/strata.tar.gz",
            listener.local_addr().expect("stub server address")
        );
        let handle = std::thread::spawn(move || {
            let Ok((mut stream, _peer)) = listener.accept() else {
                return;
            };
            use std::io::{Read, Write};
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .expect("request timeout");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).expect("read request");
                assert!(
                    count > 0 && request.len() + count <= 8192,
                    "complete bounded HTTP request"
                );
                request.extend_from_slice(&buffer[..count]);
            }
            let _written = stream.write_all(headers.as_bytes());
            let _written = stream.write_all(&body);
            let _flushed = stream.flush();
        });
        Self {
            url,
            handle: Some(handle),
        }
    }

    fn with_body(body: Vec<u8>) -> Self {
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        Self::serving(headers, body)
    }

    fn claiming(length: u64) -> Self {
        let headers =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n");
        Self::serving(headers, Vec::new())
    }
}

impl Drop for StubServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _joined = handle.join();
        }
    }
}

fn download(
    server: &StubServer,
    destination: &Path,
    limit: u64,
    cancel: &InstallCancel,
) -> Result<(), String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let outcome = download_to_file_bounded(&server.url, destination, limit, cancel, &sender);
    drop(sender);
    let _drained: Vec<_> = receiver.into_iter().collect();
    match outcome {
        Ok(()) => Ok(()),
        Err(InstallStop::Cancelled) => Err("cancelled".to_owned()),
        Err(InstallStop::Failed(message)) => Err(message),
    }
}

#[test]
fn missing_signed_metadata_offers_manual_installation() {
    let server = StubServer::serving(
        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
        Vec::new(),
    );
    let result = super::fetch_update_metadata(&server.url, 8192, &InstallCancel::new());
    assert!(
        matches!(result, Err(InstallStop::Failed(message)) if message.contains("no signed update manifest") && message.contains("manually"))
    );
}

#[test]
fn signed_metadata_download_keeps_the_exact_published_bytes() {
    let body = b"{ \"schema\": 1 }\n";
    let server = StubServer::with_body(body.to_vec());
    assert_eq!(
        super::fetch_update_metadata(&server.url, body.len() as u64, &InstallCancel::new())
            .expect("metadata download"),
        body
    );
}

#[test]
fn a_download_within_the_ceiling_is_written_to_disk() {
    let dir = scratch_dir("download-ok", line!());
    let destination = dir.join("strata.tar.gz");
    let server = StubServer::with_body(vec![b'x'; 2048]);

    download(&server, &destination, 4096, &InstallCancel::new()).expect("download should succeed");

    assert_eq!(
        fs::metadata(&destination).expect("downloaded file").len(),
        2048
    );
}

#[test]
fn a_download_advertising_more_than_the_ceiling_is_refused() {
    let dir = scratch_dir("download-claimed", line!());
    let destination = dir.join("strata.tar.gz");
    let server = StubServer::claiming(64 * 1024);

    let error = download(&server, &destination, 4096, &InstallCancel::new())
        .expect_err("an oversized content-length must be refused");

    assert!(
        error.contains("larger than expected"),
        "unexpected: {error}"
    );
    assert!(!destination.exists());
}

#[test]
fn a_download_that_streams_past_the_ceiling_is_stopped() {
    let dir = scratch_dir("download-streamed", line!());
    let destination = dir.join("strata.tar.gz");
    let server = StubServer::serving(
        "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_owned(),
        vec![b'x'; 64 * 1024],
    );

    let error = download(&server, &destination, 4096, &InstallCancel::new())
        .expect_err("an oversized stream must be stopped");

    assert!(
        error.contains("larger than expected"),
        "unexpected: {error}"
    );
    assert!(
        fs::metadata(&destination).expect("partial file").len() <= 4096 + 64 * 1024,
        "the partial download should not have been allowed to run away"
    );
}

#[test]
fn a_cancelled_download_stops_without_reporting_a_failure() {
    let dir = scratch_dir("download-cancelled", line!());
    let destination = dir.join("strata.tar.gz");
    let server = StubServer::with_body(vec![b'x'; 2048]);
    let cancel = InstallCancel::new();
    cancel.cancel();

    let error = download(&server, &destination, 4096, &cancel).expect_err("cancel must stop");

    assert_eq!(error, "cancelled");
}

#[test]
fn a_preserved_executable_is_restored_after_a_failed_replacement() {
    let dir = scratch_dir("rollback", line!());
    let installed = dir.join("strata");
    fs::write(&installed, b"previous version").expect("write installed binary");

    let rollback = stage_rollback(&installed, &dir).expect("stage rollback");
    fs::write(&installed, b"broken replacement").expect("replace installed binary");
    restore_rollback(&rollback, &installed).expect("restore rollback");

    assert_eq!(
        fs::read(&installed).expect("read restored binary"),
        b"previous version"
    );
    assert!(!rollback.exists(), "the rollback copy should be consumed");
}

#[test]
fn a_staged_binary_that_cannot_run_is_rejected_before_installation() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch_dir("staged-binary", line!());
    let staged = dir.join("strata");
    fs::write(&staged, "#!/bin/sh\nexit 1\n").expect("write staged binary");
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).expect("make executable");

    assert!(verify_staged_binary(&staged).is_err());
}

#[test]
fn a_staged_binary_that_runs_is_accepted() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch_dir("staged-binary-ok", line!());
    let staged = dir.join("strata");
    fs::write(&staged, "#!/bin/sh\nexit 0\n").expect("write staged binary");
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).expect("make executable");

    assert!(verify_staged_binary(&staged).is_ok());
}
