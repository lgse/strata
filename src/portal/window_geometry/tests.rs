// SPDX-License-Identifier: MIT

mod centering;

use super::*;
use std::os::unix::net::UnixListener;

fn window(class: &str, width: i32, height: i32) -> serde_json::Value {
    serde_json::json!({
        "class": class,
        "initialClass": class,
        "mapped": true,
        "hidden": false,
        "size": [width, height]
    })
}

fn size(windows: Vec<serde_json::Value>, app_id: &str) -> Option<(i32, i32)> {
    application_window_size(
        &serde_json::to_vec(&windows).expect("serialize windows"),
        app_id,
    )
}

#[test]
fn unique_requesting_application_uses_logical_window_size() {
    assert_eq!(
        size(
            vec![window("zen", 960, 1000), window("terminal", 960, 1000)],
            "zen"
        ),
        Some((960, 1000))
    );
    assert_eq!(
        size(
            vec![window("org.mozilla.Firefox", 1920, 1080)],
            "org.mozilla.firefox"
        ),
        Some((1920, 1080))
    );
    let mut renamed = window("dynamic-class", 800, 600);
    renamed["initialClass"] = "zen".into();
    assert_eq!(size(vec![renamed], "zen"), Some((800, 600)));
}

#[test]
fn ambiguous_missing_and_invalid_windows_have_no_hint() {
    assert_eq!(
        size(
            vec![window("zen", 960, 1000), window("zen", 800, 600)],
            "zen"
        ),
        None
    );
    assert_eq!(size(vec![window("terminal", 960, 1000)], "zen"), None);
    assert_eq!(
        size(vec![window("zen", 960, 1000)], "org.example.zen"),
        None
    );
    assert_eq!(size(vec![window("", 960, 1000)], ""), None);
    assert_eq!(size(vec![], "zen"), None);
    for dimensions in [(0, 1000), (960, 0), (-1, 500)] {
        assert_eq!(
            size(vec![window("zen", dimensions.0, dimensions.1)], "zen"),
            None
        );
    }
    for (property, value) in [("mapped", false), ("hidden", true)] {
        let mut unavailable = window("zen", 960, 1000);
        unavailable[property] = value.into();
        assert_eq!(size(vec![unavailable.clone()], "zen"), None);
        assert_eq!(
            size(vec![unavailable, window("zen", 800, 600)], "zen"),
            None
        );
    }
    for reply in [b"not json".as_slice(), b"{}", b"[{}]"] {
        assert_eq!(application_window_size(reply, "zen"), None);
    }
}

#[test]
fn socket_paths_stay_inside_the_runtime_instance() {
    assert_eq!(
        socket_path(Path::new("/run/user/1000"), "abc_123-def"),
        Some(PathBuf::from(
            "/run/user/1000/hypr/abc_123-def/.socket.sock"
        ))
    );
    for signature in ["", "..", "../other", "/tmp/socket", "a/b", "a\0b"] {
        assert_eq!(socket_path(Path::new("/run/user/1000"), signature), None);
    }
    assert_eq!(socket_path(Path::new("relative"), "abc"), None);
}

fn query_fixture(reply: Vec<u8>, delay: Duration, timeout: Duration) -> Option<(i32, i32)> {
    let root = tempfile::tempdir().expect("temporary socket directory");
    let socket = root.path().join("fixture.sock");
    let listener =
        Async::new(UnixListener::bind(&socket).expect("bind fixture")).expect("async listener");
    async_io::block_on(async {
        let (result, ()) = future::zip(query_size(&socket, "zen", timeout), async {
            let (mut stream, _) = listener.accept().await.expect("accept fixture");
            let mut command = [0; 9];
            stream.read_exact(&mut command).await.expect("read query");
            assert_eq!(&command, b"j/clients");
            Timer::after(delay).await;
            let _ = stream.write_all(&reply).await;
        })
        .await;
        result
    })
}

#[test]
fn compositor_query_is_bounded_and_optional() {
    let reply = serde_json::to_vec(&vec![window("zen", 960, 1000)]).expect("serialize fixture");
    assert_eq!(
        query_fixture(reply.clone(), Duration::ZERO, Duration::from_secs(2)),
        Some((960, 1000))
    );
    assert_eq!(
        query_fixture(reply, Duration::from_millis(60), Duration::from_millis(30)),
        None
    );
    assert_eq!(
        query_fixture(
            vec![b' '; MAX_REPLY_BYTES as usize + 1],
            Duration::ZERO,
            Duration::from_secs(2)
        ),
        None
    );
    assert_eq!(
        query_fixture(b"invalid".to_vec(), Duration::ZERO, Duration::from_secs(2)),
        None
    );
    let root = tempfile::tempdir().expect("temporary directory");
    assert_eq!(
        async_io::block_on(query_size(
            &root.path().join("missing"),
            "zen",
            QUERY_TIMEOUT
        )),
        None
    );
}

#[test]
fn requests_without_identifiers_do_not_query_the_compositor() {
    async_io::block_on(async {
        let app_id = MaybeAppID::from("zen");
        assert_eq!(parent_size_hint(Some(&app_id), None).await, None);
        assert_eq!(parent_size_hint(None, None).await, None);
    });
}
