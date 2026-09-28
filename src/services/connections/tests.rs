// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::*;

fn draft(name: &str, uri: &str) -> ConnectionDraft {
    ConnectionDraft {
        name: name.into(),
        destination: RemoteDestination::parse(uri).expect("valid destination"),
    }
}

fn form(protocol: RemoteProtocol, server: &str) -> ConnectionForm {
    ConnectionForm {
        protocol: Some(protocol),
        server: server.into(),
        ..ConnectionForm::default()
    }
}

fn written(store: &ConnectionStore) -> Value {
    serde_json::from_slice(&store.to_json().expect("writable store")).expect("valid JSON")
}

#[test]
fn saved_connections_round_trip_with_a_versioned_sanitized_record() {
    let mut store = ConnectionStore::empty();
    let saved = store
        .add_with_id(
            draft("Backups", "sftp://alice:hunter2@Server:2222/srv/"),
            "id-1".into(),
        )
        .expect("added");
    assert_eq!(
        saved.destination().canonical_uri(),
        "sftp://alice@server:2222/srv"
    );

    let file = written(&store);
    assert_eq!(
        file,
        json!({
            "version": 1,
            "connections": [{
                "id": "id-1",
                "name": "Backups",
                "protocol": "sftp",
                "uri": "sftp://alice@server:2222/srv",
            }],
        })
    );
    let reloaded = ConnectionStore::parse(&store.to_json().expect("should succeed"));
    assert_eq!(reloaded.connections(), vec![saved]);
}

#[test]
fn secrets_never_reach_the_connection_file() {
    let mut store = ConnectionStore::parse(
        br#"{"version":1,"password":"top","connections":[
            {"id":"a","name":"A","protocol":"ftp","uri":"ftp://bob:pw@host/pub",
             "passphrase":"x","auth_token":"y","colour":"blue"},
            {"id":"b","name":"B","protocol":"nfs","uri":"nfs://host/","secret":"z"}
        ]}"#,
    );
    store
        .add_with_id(draft("C", "smb://carol;password=pw@nas/share"), "c".into())
        .expect("added");
    let text = String::from_utf8(store.to_json().expect("should succeed")).expect("should succeed");
    for secret in [
        "top",
        "\"pw\"",
        ":pw@",
        "passphrase",
        "auth_token",
        "\"z\"",
        "password",
    ] {
        assert!(!text.contains(secret), "{secret:?} leaked into {text}");
    }
    assert!(
        text.contains("\"colour\": \"blue\""),
        "unknown fields survive: {text}"
    );
}

#[test]
fn unknown_fields_entries_and_legacy_files_are_preserved() {
    let store = ConnectionStore::parse(
        br#"{"connections":[
            {"id":"a","name":"A","protocol":"sftp","uri":"sftp://host/","pinned":true},
            {"id":"b","name":"Future","protocol":"s3","uri":"s3://bucket/"},
            "not an object"
        ],"sort":"name"}"#,
    );
    assert_eq!(
        store.read_only_reason(),
        None,
        "unversioned files are version 1"
    );
    assert_eq!(store.connections().len(), 1);
    let file = written(&store);
    assert_eq!(file["version"], 1);
    assert_eq!(file["sort"], "name");
    assert_eq!(file["connections"][0]["pinned"], true);
    assert_eq!(file["connections"][1]["protocol"], "s3");
    assert_eq!(file["connections"][2], "not an object");
}

#[test]
fn newer_and_unreadable_files_are_never_rewritten() {
    let newer = ConnectionStore::parse(
        br#"{"version":7,"connections":[{"id":"a","name":"A","protocol":"sftp","uri":"sftp://host/"}]}"#,
    );
    assert_eq!(newer.connections().len(), 1, "newer files stay browsable");
    assert_eq!(
        newer.read_only_reason(),
        Some(&ConnectionStoreError::NewerVersion(7))
    );
    assert_eq!(newer.to_json(), Err(ConnectionStoreError::NewerVersion(7)));
    let mut newer = newer;
    assert!(matches!(
        newer.add(draft("B", "sftp://other/")),
        Err(ConnectionStoreError::NewerVersion(7))
    ));
    assert!(matches!(
        newer.rename("a", "Renamed"),
        Err(ConnectionStoreError::NewerVersion(7))
    ));

    for contents in [
        &b"{broken"[..],
        b"[]",
        br#"{"version":"one"}"#,
        br#"{"connections":{}}"#,
    ] {
        let store = ConnectionStore::parse(contents);
        assert!(
            matches!(
                store.read_only_reason(),
                Some(ConnectionStoreError::Unreadable(_))
            ),
            "{:?}",
            String::from_utf8_lossy(contents)
        );
        assert!(store.to_json().is_err());
    }
}

#[test]
fn exact_duplicates_are_rejected_but_distinct_destinations_are_allowed() {
    let mut store = ConnectionStore::empty();
    store
        .add(draft("Data", "sftp://alice@host/data"))
        .expect("added");
    for duplicate in ["sftp://alice@HOST:22/data/", "sftp://alice@host//data/."] {
        assert_eq!(
            store.add(draft("Again", duplicate)),
            Err(ConnectionStoreError::Duplicate("Data".into())),
            "{duplicate:?}"
        );
    }
    for distinct in [
        "sftp://bob@host/data",
        "sftp://host/data",
        "sftp://alice@host:2222/data",
        "sftp://alice@host/other",
        "ftps://alice@host/data",
    ] {
        store
            .add(draft(distinct, distinct))
            .unwrap_or_else(|error| {
                panic!("{distinct:?} should be allowed: {error}");
            });
    }
}

#[test]
fn rename_edit_and_remove_change_only_the_named_record() {
    let mut store = ConnectionStore::empty();
    let first = store
        .add(draft("First", "smb://nas/media"))
        .expect("should succeed");
    let second = store
        .add(draft("Second", "smb://nas/backup"))
        .expect("should succeed");

    let renamed = store
        .rename(&first.id, "  Movies  ")
        .expect("should succeed");
    assert_eq!(renamed.name, "Movies");
    assert_eq!(renamed.destination(), first.destination());
    assert_eq!(
        store.rename(&first.id, "\n"),
        Err(ConnectionStoreError::InvalidName("Enter a name."))
    );

    assert_eq!(
        store.update(&second.id, draft("Second", "smb://NAS/Media")),
        Err(ConnectionStoreError::Duplicate("Movies".into()))
    );
    let edited = store
        .update(&second.id, draft("Archive", "smb://nas/archive"))
        .expect("should succeed");
    assert_eq!(edited.id, second.id);
    assert_eq!(
        store.update(&first.id, draft("Movies", "smb://nas/media/")),
        Ok(SavedConnection {
            name: "Movies".into(),
            ..first.clone()
        }),
        "editing a record may keep its own destination"
    );

    assert_eq!(
        store.remove(&first.id).expect("should succeed").name,
        "Movies"
    );
    assert_eq!(store.remove(&first.id), Err(ConnectionStoreError::NotFound));
    assert_eq!(store.connections(), vec![edited]);
}

#[test]
fn saved_connections_find_locations_they_contain() {
    let mut store = ConnectionStore::empty();
    let share = store
        .add(draft("Share", "smb://nas/share"))
        .expect("should succeed");
    assert_eq!(
        store.containing(&Location::uri("smb://nas/share/folder/deeper")),
        Some(share)
    );
    assert_eq!(store.containing(&Location::uri("smb://nas/other")), None);
    assert_eq!(store.containing(&Location::local("/tmp")), None);
}

#[test]
fn forms_build_sanitized_destinations_for_every_protocol() {
    let cases = [
        (
            ConnectionForm {
                share: "Media".into(),
                path: "/Movies/".into(),
                username: "alice".into(),
                ..form(RemoteProtocol::Smb, "NAS")
            },
            "smb://alice@nas/Media/Movies",
            "Movies on nas",
        ),
        (
            ConnectionForm {
                port: "2222".into(),
                path: "srv/data".into(),
                ..form(RemoteProtocol::Sftp, "host")
            },
            "sftp://host:2222/srv/data",
            "data on host",
        ),
        (
            form(RemoteProtocol::Ftp, "ftp.example.com"),
            "ftp://ftp.example.com/",
            "ftp.example.com",
        ),
        (
            ConnectionForm {
                port: "21".into(),
                ..form(RemoteProtocol::Ftps, "host")
            },
            "ftps://host/",
            "host",
        ),
        (
            ConnectionForm {
                path: "/dav".into(),
                ..form(RemoteProtocol::Dav, "host")
            },
            "dav://host/dav",
            "dav on host",
        ),
        (
            ConnectionForm {
                path: "/my files".into(),
                name: "Cloud".into(),
                ..form(RemoteProtocol::Davs, "host")
            },
            "davs://host/my%20files",
            "Cloud",
        ),
    ];
    for (form, uri, name) in cases {
        let draft = form
            .validate()
            .unwrap_or_else(|error| panic!("{form:?}: {error:?}"));
        assert_eq!(draft.destination.canonical_uri(), uri);
        assert_eq!(draft.name, name);
    }
}

#[test]
fn webdav_forms_normalize_https_endpoints_to_davs() {
    let draft = form(RemoteProtocol::Dav, "https://cloud.example/remote.php/dav/")
        .validate()
        .expect("HTTPS endpoint");
    assert_eq!(
        draft.destination.canonical_uri(),
        "davs://cloud.example/remote.php/dav"
    );
    let plain = form(RemoteProtocol::Davs, "http://cloud.example:8080/dav")
        .validate()
        .expect("HTTP endpoint");
    assert_eq!(
        plain.destination.canonical_uri(),
        "dav://cloud.example:8080/dav"
    );
    assert_eq!(
        form(RemoteProtocol::Sftp, "https://cloud.example/")
            .validate()
            .expect_err("should be rejected")
            .field,
        ConnectionFormField::Server
    );
}

#[test]
fn forms_reject_invalid_fields_and_embedded_passwords() {
    use ConnectionFormField as Field;
    let cases = [
        (form(RemoteProtocol::Sftp, "  "), Field::Server),
        (form(RemoteProtocol::Sftp, "host/path"), Field::Server),
        (
            form(RemoteProtocol::Sftp, "sftp://bob:pw@host/"),
            Field::Server,
        ),
        (
            ConnectionForm {
                port: "70000".into(),
                ..form(RemoteProtocol::Sftp, "host")
            },
            Field::Port,
        ),
        (
            ConnectionForm {
                port: "0".into(),
                ..form(RemoteProtocol::Sftp, "host")
            },
            Field::Port,
        ),
        (
            ConnectionForm {
                username: "bob:pw".into(),
                ..form(RemoteProtocol::Sftp, "host")
            },
            Field::Username,
        ),
        (form(RemoteProtocol::Smb, "nas"), Field::Share),
        (
            ConnectionForm {
                share: "a/b".into(),
                ..form(RemoteProtocol::Smb, "nas")
            },
            Field::Share,
        ),
        (
            ConnectionForm {
                path: "/srv/../..".into(),
                ..form(RemoteProtocol::Sftp, "host")
            },
            Field::Path,
        ),
        (
            ConnectionForm {
                name: "two\nlines".into(),
                ..form(RemoteProtocol::Sftp, "host")
            },
            Field::Name,
        ),
    ];
    for (form, field) in cases {
        assert_eq!(
            form.validate().expect_err("should be rejected").field,
            field,
            "{form:?}"
        );
    }
    assert_eq!(
        ConnectionForm::default()
            .validate()
            .expect_err("should be rejected")
            .field,
        Field::Server
    );
}

#[test]
fn edit_forms_round_trip_saved_connections() {
    let mut store = ConnectionStore::empty();
    for uri in [
        "smb://alice@nas/Media/Movies",
        "sftp://host:2222/srv/data",
        "davs://host/my%20files",
        "ftp://host/",
    ] {
        let saved = store.add(draft(uri, uri)).expect("should succeed");
        let form = ConnectionForm::from_connection(&saved);
        let rebuilt = form.validate().expect("should succeed");
        assert!(
            rebuilt.destination.same_destination(saved.destination()),
            "{uri}: {:?}",
            rebuilt.destination.canonical_uri()
        );
        assert_eq!(rebuilt.name, saved.name);
    }
}
