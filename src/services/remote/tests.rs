// SPDX-License-Identifier: MIT

use super::*;

fn destination(uri: &str) -> RemoteDestination {
    RemoteDestination::parse(uri).unwrap_or_else(|| panic!("{uri:?} should parse"))
}

#[test]
fn canonical_destinations_normalize_every_identity_part() {
    for (input, canonical) in [
        (
            "SFTP://Server.Example.COM/home/",
            "sftp://server.example.com/home",
        ),
        ("sftp://server:22/home", "sftp://server/home"),
        ("sftp://server:2222/home", "sftp://server:2222/home"),
        ("ftps://server:21/", "ftps://server/"),
        ("davs://server:443/dav", "davs://server/dav"),
        ("dav://server:80/dav/", "dav://server/dav"),
        (
            "smb://server:445/share//folder/",
            "smb://server/share/folder",
        ),
        ("sftp://server/a/./b/../c", "sftp://server/a/c"),
        (
            "sftp://server/%7euser/%2fslash",
            "sftp://server/~user/%2Fslash",
        ),
        ("sftp://server/with%20space", "sftp://server/with%20space"),
        ("sftp://alice@server/", "sftp://alice@server/"),
        ("sftp://alice:secret@server/", "sftp://alice@server/"),
        (
            "smb://alice;password=secret@server/share",
            "smb://alice@server/share",
        ),
        ("sftp://[FE80::1]:2222/", "sftp://[fe80::1]:2222/"),
        ("sftp://server.example./", "sftp://server.example/"),
    ] {
        assert_eq!(destination(input).canonical_uri(), canonical, "{input:?}");
    }
}

#[test]
fn destinations_without_a_host_or_supported_protocol_are_rejected() {
    for uri in [
        "sftp:///path",
        "https://server/dav",
        "file:///home",
        "network:///",
        "not a uri",
    ] {
        assert_eq!(RemoteDestination::parse(uri), None, "{uri:?}");
    }
}

#[test]
fn duplicate_identity_distinguishes_users_ports_and_destinations() {
    let base = destination("sftp://alice@server/data");
    for same in [
        "SFTP://alice@SERVER:22/data/",
        "sftp://alice@server/data/./",
        "sftp://alice@server//data",
    ] {
        assert!(base.same_destination(&destination(same)), "{same:?}");
    }
    for different in [
        "sftp://bob@server/data",
        "sftp://server/data",
        "sftp://alice@server:2222/data",
        "sftp://alice@server/Data",
        "sftp://alice@other/data",
        "ftps://alice@server/data",
    ] {
        assert!(
            !base.same_destination(&destination(different)),
            "{different:?}"
        );
    }
    assert!(
        destination("smb://server/Share/Folder")
            .same_destination(&destination("smb://SERVER/share/folder")),
        "SMB shares and folders compare case-insensitively"
    );
}

#[test]
fn mount_matching_reuses_mounts_that_serve_the_destination() {
    let mount = destination("sftp://alice@server/");
    assert!(destination("sftp://alice@server/srv/data").is_served_by(&mount));
    assert!(
        destination("sftp://server/srv/data").is_served_by(&mount),
        "a destination without a user reuses any user's mount"
    );
    assert!(!destination("sftp://bob@server/srv").is_served_by(&mount));
    assert!(!destination("sftp://alice@server:2222/srv").is_served_by(&mount));

    let share = destination("smb://server/share");
    assert!(destination("smb://server/SHARE/folder").is_served_by(&share));
    assert!(!destination("smb://server/shared").is_served_by(&share));
    assert!(!destination("smb://server/other").is_served_by(&share));

    let rebased = destination("sftp://server/srv/data").on_mount(&mount);
    assert_eq!(rebased.canonical_uri(), "sftp://alice@server/srv/data");
}

#[test]
fn server_keys_ignore_the_path() {
    assert_eq!(
        destination("ftp://alice@server:2121/pub/a").server_key(),
        destination("ftp://alice@server:2121/pub/b").server_key(),
    );
    assert_ne!(
        destination("ftp://alice@server/pub").server_key(),
        destination("ftp://bob@server/pub").server_key(),
    );
}

#[test]
fn default_names_describe_the_destination() {
    assert_eq!(
        destination("smb://nas/media").default_name(),
        "media on nas"
    );
    assert_eq!(destination("sftp://host/").default_name(), "host");
    assert_eq!(
        destination("davs://cloud/remote.php/dav/files/alice%20b").default_name(),
        "alice b on cloud"
    );
}

#[test]
fn webdav_endpoints_map_http_schemes_to_gvfs_schemes() {
    assert_eq!(
        webdav_uri_for_endpoint("https://cloud/remote.php/dav").as_deref(),
        Some("davs://cloud/remote.php/dav")
    );
    assert_eq!(
        webdav_uri_for_endpoint("HTTP://cloud:8080/dav").as_deref(),
        Some("dav://cloud:8080/dav")
    );
    assert_eq!(webdav_uri_for_endpoint("ftp://cloud/"), None);
    assert!(web_address_hint("https").is_some());
    assert!(web_address_hint("sftp").is_none());
}

#[test]
fn backend_messages_name_the_protocol_without_a_single_distribution_package() {
    let smb = backend_unavailable_message("smb://host/share");
    assert!(smb.contains("smb://") && smb.contains("gvfs-smb") && smb.contains("gvfs-backends"));
    for uri in ["sftp://host/", "ftp://host/", "davs://host/"] {
        let message = backend_unavailable_message(uri);
        assert!(message.contains("gvfs-backends"), "{message}");
        assert!(!message.contains("host/"), "{message}");
    }
    assert!(backend_unavailable_message("afp://host/").contains("afp://"));
}

#[test]
fn network_discovery_states_are_distinct_and_keep_direct_entry() {
    let network = Location::uri(NETWORK_ROOT_URI);
    let empty = directory_empty_text(Some(&network), &["dns-sd", "sftp"]);
    let missing = directory_empty_text(Some(&network), &["sftp", "ftp"]);
    let failed = load_failure_message(
        &network,
        &glib::Error::new(gio::IOErrorEnum::TimedOut, "timed out"),
    );
    let unavailable = load_failure_message(
        &network,
        &glib::Error::new(gio::IOErrorEnum::NotSupported, "Operation not supported"),
    );
    assert!(empty.contains("No computers or shared folders"));
    assert!(missing.contains("no GVfs discovery backend"));
    assert!(failed.contains("failed or was blocked"));
    assert!(unavailable.contains("isn't installed or running"));
    for text in [&empty, &missing, &failed, &unavailable] {
        assert!(text.contains("Ctrl+L"), "{text}");
    }
    assert_eq!(
        directory_empty_text(Some(&Location::local("/tmp")), &["dns-sd"]),
        "This directory is empty"
    );
    assert_eq!(directory_failure_text(Some(&network), &failed), failed);
}

#[test]
fn a_disconnected_remote_column_is_reported_as_unavailable() {
    let location = Location::uri("sftp://host/srv");
    let message = load_failure_message(
        &location,
        &glib::Error::new(gio::IOErrorEnum::NotMounted, "not mounted"),
    );
    assert_eq!(message, RemoteFailure::Disconnected.guidance(None));
    let text = directory_failure_text(Some(&location), &message);
    assert!(text.starts_with("This location is unavailable"));
    assert!(text.contains("Retry"));
}

#[test]
fn remote_errors_map_to_actionable_failures() {
    use gio::IOErrorEnum as Io;
    let cases = [
        (Io::Cancelled, "cancelled", RemoteFailure::Cancelled),
        (
            Io::FailedHandled,
            "Password dialog cancelled",
            RemoteFailure::Cancelled,
        ),
        (
            Io::NotSupported,
            "Operation not supported",
            RemoteFailure::BackendMissing,
        ),
        (
            Io::HostNotFound,
            "Hostname not known",
            RemoteFailure::HostNotFound,
        ),
        (Io::Failed, "No route to host", RemoteFailure::HostNotFound),
        (
            Io::ConnectionRefused,
            "Connection refused by server",
            RemoteFailure::ConnectionRefused,
        ),
        (
            Io::TimedOut,
            "Timed out when logging in",
            RemoteFailure::TimedOut,
        ),
        (
            Io::Failed,
            "Host key verification failed",
            RemoteFailure::HostKeyRejected,
        ),
        (
            Io::Failed,
            "Unacceptable TLS certificate",
            RemoteFailure::CertificateRejected,
        ),
        (
            Io::Failed,
            "Too many authentication failures",
            RemoteFailure::AuthenticationFailed,
        ),
        (
            Io::Failed,
            "Failed to mount Windows share: Permission denied",
            RemoteFailure::AuthenticationFailed,
        ),
        (
            Io::PermissionDenied,
            "Permission denied",
            RemoteFailure::AuthenticationFailed,
        ),
        (Io::NotMounted, "not mounted", RemoteFailure::Disconnected),
        (Io::Busy, "Target is busy", RemoteFailure::Busy),
        (Io::Failed, "Invalid reply received", RemoteFailure::Other),
    ];
    for (code, message, expected) in cases {
        let error = glib::Error::new(code, message);
        assert_eq!(
            classify_remote_error(&error, RemoteErrorContext::Mount),
            expected,
            "{message:?}"
        );
    }
    let tls = glib::Error::new(gio::TlsError::BadCertificate, "bad certificate");
    assert_eq!(
        classify_remote_error(&tls, RemoteErrorContext::Mount),
        RemoteFailure::CertificateRejected
    );
    let denied = glib::Error::new(Io::PermissionDenied, "Permission denied");
    assert_eq!(
        classify_remote_error(&denied, RemoteErrorContext::Browse),
        RemoteFailure::PermissionDenied,
        "browsing a readable server is a permission problem, not a sign-in problem"
    );
}

#[test]
fn failure_guidance_never_repeats_endpoint_details() {
    let error = glib::Error::new(
        gio::IOErrorEnum::HostNotFound,
        "sftp://alice@secret-host/private: Hostname not known",
    );
    let message = load_failure_message(&Location::uri("sftp://alice@secret-host/private"), &error);
    assert!(!message.contains("secret-host") && !message.contains("alice"));
    assert_eq!(
        redact_endpoints("Error opening sftp://alice@secret-host/private now"),
        "Error opening sftp://… now"
    );
}

#[test]
fn a_decline_during_the_operation_resolves_as_cancelled() {
    let rejected = Err(glib::Error::new(
        gio::IOErrorEnum::Failed,
        "Host key verification failed",
    ));
    assert_eq!(
        resolve_mount_result(&rejected, true, RemoteErrorContext::Mount, true),
        MountResolution::Cancelled
    );
    assert_eq!(
        resolve_mount_result(&rejected, false, RemoteErrorContext::Mount, true),
        MountResolution::Failed(RemoteFailure::HostKeyRejected),
        "a changed host key the user was never asked about is a failure"
    );
    let already = Err(glib::Error::new(
        gio::IOErrorEnum::AlreadyMounted,
        "mounted",
    ));
    assert_eq!(
        resolve_mount_result(&already, false, RemoteErrorContext::Mount, true),
        MountResolution::Succeeded
    );
    assert_eq!(
        resolve_mount_result(&Ok(()), true, RemoteErrorContext::Mount, true),
        MountResolution::Succeeded
    );
    let refused = Err(glib::Error::new(
        gio::IOErrorEnum::NotSupported,
        "500 OOPS: server refused",
    ));
    assert_eq!(
        resolve_mount_result(&refused, false, RemoteErrorContext::Mount, false),
        MountResolution::Failed(RemoteFailure::BackendMissing)
    );
    assert_eq!(
        resolve_mount_result(&refused, false, RemoteErrorContext::Mount, true),
        MountResolution::Failed(RemoteFailure::Other),
        "an installed backend's server rejection isn't a missing backend"
    );
}

#[test]
fn trust_questions_are_classified_and_never_default_to_acceptance() {
    let host = MountQuestion::classify(
        "Identity Verification Failed\nVerifying the identity of “host” failed, this happens \
         when you log in to a computer the first time.\n\nThe identity sent by the remote \
         computer is “SHA256:abc”.",
        &["Log In Anyway".into(), "Cancel Login".into()],
    );
    assert_eq!(host.kind, MountQuestionKind::HostIdentity);
    assert!(host.detail.starts_with("Verifying the identity"));
    assert!(host.detail.contains("SHA256:abc"));
    assert!(host.is_risky_choice(0) && !host.is_risky_choice(1));

    let certificate = MountQuestion::classify(
        "Identity Verification Failed\n\n\tThe signing certificate authority is not known.\n\n\
         Certificate information:\n\tIdentity: host\n\nAre you really sure you would like to \
         continue?",
        &["Yes".into(), "No".into()],
    );
    assert_eq!(certificate.kind, MountQuestionKind::Certificate);
    assert!(
        certificate
            .detail
            .contains("certificate authority is not known")
    );
    assert!(certificate.is_risky_choice(0));

    let other = MountQuestion::classify("Replace the file?", &["Replace".into(), "Skip".into()]);
    assert_eq!(other.kind, MountQuestionKind::Other);
    assert_eq!(other.detail, "Replace the file?");
    assert!(!other.is_risky_choice(0));
}

#[test]
fn only_plaintext_protocols_need_a_transport_warning() {
    for (uri, warned) in [
        ("ftp://host/pub", true),
        ("dav://host/dav", true),
        ("ftps://host/pub", false),
        ("davs://host/dav", false),
        ("sftp://host/", false),
        ("smb://host/share", false),
    ] {
        assert_eq!(
            plaintext_destination(&Location::uri(uri)).is_some(),
            warned,
            "{uri}"
        );
    }
    assert_eq!(plaintext_destination(&Location::local("/tmp")), None);
    let warning = plaintext_warning_text(RemoteProtocol::Ftp);
    assert!(warning.contains("doesn't encrypt"));
    assert!(warning.contains("certificate checks"));
    assert_eq!(
        plaintext_secure_alternative(RemoteProtocol::Dav),
        Some(RemoteProtocol::Davs)
    );
}

#[test]
fn every_remote_protocol_gets_network_presentation() {
    for protocol in RemoteProtocol::ALL {
        let root = format!("{}://host/", protocol.scheme());
        assert_eq!(RemoteProtocol::for_uri(&root), Some(protocol));
    }
    assert_eq!(RemoteProtocol::for_uri("file:///media/usb"), None);
    assert_eq!(RemoteProtocol::for_uri("mtp://phone/"), None);
}
