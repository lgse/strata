// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn camera_thumbnail_bindings_keep_duplicate_names_distinct_and_discard_stale_results() {
    crate::test_support::gtk_test(
        "ui::thumbnail::camera::tests::camera_thumbnail_bindings_keep_duplicate_names_distinct_and_discard_stale_results",
        || {
            crate::ui::theme::ThemeManager::shared();
            for extension in ["JPG", "HEIC", "HEIF", "MOV"] {
                let name = format!("IMG_0001.{extension}");
                clear_thumbnail_runtime();
                hold_thumbnail_workers();
                let images = [ThumbnailSlot::new(64), ThumbnailSlot::new(64)];
                let uris = [
                    format!("gphoto2://camera/202606/{name}"),
                    format!("gphoto2://camera/202605/{name}"),
                ];
                let paths = uris.each_ref().map(String::as_str);
                for (image, uri) in images.iter().zip(paths) {
                    let entry = FileEntry {
                        location: crate::model::Location::uri(uri),
                        thumbnail_path: None,
                        native_name: name.clone().into(),
                        display_name: name.clone(),
                        kind: crate::model::EntryKind::File,
                        size: MetadataValue::Unknown,
                        modified_unix_seconds: MetadataValue::Unknown,
                        mode: MetadataValue::Unavailable,
                        is_hidden: false,
                        image_dimensions: MetadataValue::Unknown,
                        child_count: MetadataValue::Unknown,
                        duration_seconds: MetadataValue::Unknown,
                    };
                    set_thumbnail_or_icon(image, &entry, crate::assets::icons::PICTURES, 32, 64);
                }
                while glib::MainContext::default().iteration(false) {}
                fire_settled_thumbnails();
                let results = PENDING_THUMBNAILS.with(|pending| {
                    let pending = pending.borrow();
                    assert_eq!(
                        pending.len(),
                        2,
                        "queued keys: {:?}; active {}",
                        pending.keys().map(|key| &key.path).collect::<Vec<_>>(),
                        ACTIVE_REQUESTS.with(|requests| requests.borrow().len())
                    );
                    paths.map(|uri| {
                        let (key, request) = pending
                            .iter()
                            .find(|(key, _)| key.path == Path::new(uri))
                            .expect("distinct camera job without waiting for metadata");
                        assert_eq!(request.kind, ThumbnailKind::Camera);
                        (key.clone(), request.id)
                    })
                });
                let stale =
                    take_pending_targets(&results[0].0, results[0].1).expect("first targets");
                let live =
                    take_pending_targets(&results[1].0, results[1].1).expect("second targets");
                show_fallback_icon(&images[0], crate::assets::icons::FOLDER, 32);
                let pixels = glib::Bytes::from_owned(vec![255u8; 4]);
                let texture =
                    gdk::MemoryTexture::new(1, 1, gdk::MemoryFormat::R8g8b8a8, &pixels, 4)
                        .upcast::<gdk::Texture>();
                finish_thumbnail_targets(stale, Some(&texture), Path::new(paths[0]));
                finish_thumbnail_targets(live, Some(&texture), Path::new(paths[1]));
                assert!(!displayed_thumbnail_matches(
                    &images[0],
                    Path::new(paths[0])
                ));
                assert!(displayed_thumbnail_matches(&images[1], Path::new(paths[1])));
                clear_thumbnail_runtime();
            }
        },
    );
}

#[test]
#[expect(
    unsafe_code,
    reason = "Verify the null content-type fixture through the underlying GIO contract"
)]
fn camera_icon_without_content_type_loads_without_panicking() {
    use glib::translate::*;
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    let bytes = glib::Bytes::from_static(b"camera preview payload");
    let icon = gio::BytesIcon::new(&bytes).upcast::<gio::LoadableIcon>();
    // BytesIcon reproduces the real GVfs contract: a successful stream with no MIME type.
    let mut content_type = std::ptr::null_mut();
    let mut error = std::ptr::null_mut();
    // SAFETY: the icon is live and the optional output pointers are valid.
    let raw_stream = unsafe {
        gio::ffi::g_loadable_icon_load(
            icon.to_glib_none().0,
            256,
            &mut content_type,
            std::ptr::null_mut(),
            &mut error,
        )
    };
    // SAFETY: GIO transfers ownership of each nullable output to this caller.
    let stream: Option<gio::InputStream> = unsafe { from_glib_full(raw_stream) };
    // SAFETY: the optional type string is a full-transfer GIO output.
    let content_type: Option<glib::GString> = unsafe { from_glib_full(content_type) };
    // SAFETY: the optional error is a full-transfer GIO output.
    let error: Option<glib::Error> = unsafe { from_glib_full(error) };
    assert!(stream.is_some());
    assert!(error.is_none());
    assert!(content_type.is_none());
    context.block_on(async {
        let stream = load_icon_stream(&icon)
            .await
            .expect("nullable type is valid");
        assert_eq!(
            read_icon(&stream, MAX_ICON_BYTES)
                .await
                .expect("icon bytes"),
            bytes.as_ref()
        );
        let fixture = tempfile::tempdir().expect("fixture");
        let missing = gio::FileIcon::new(&gio::File::for_path(fixture.path().join("missing")))
            .upcast::<gio::LoadableIcon>();
        assert!(
            load_icon_stream(&missing)
                .await
                .expect_err("failed load must stay an error")
                .matches(gio::IOErrorEnum::NotFound)
        );
    });
}

#[test]
fn camera_icon_load_callback_survives_a_dropped_future() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    context.block_on(async {
        let icon = gio::BytesIcon::new(&glib::Bytes::from_static(b"preview"))
            .upcast::<gio::LoadableIcon>();
        let weak = icon.downgrade();
        {
            let mut future = Box::pin(load_icon_stream(&icon));
            assert!(futures_lite::future::poll_once(&mut future).await.is_none());
        }
        drop(icon);
        let deadline = Instant::now() + Duration::from_secs(2);
        while weak.upgrade().is_some() && Instant::now() < deadline {
            glib::timeout_future(Duration::from_millis(1)).await;
        }
        assert!(
            weak.upgrade().is_none(),
            "cancelled callback must release its source"
        );
    });
}

#[test]
fn camera_icon_reads_are_bounded_even_without_size_metadata() {
    let _lock = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("context lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire().expect("context owner");
    for (size, limit, succeeds) in [(0, 0, true), (8193, 8193, true), (8193, 8192, false)] {
        let input = vec![7; size];
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(input.clone()));
        let result = context.block_on(read_icon(&stream, limit));
        assert_eq!(result.is_ok(), succeeds);
        if succeeds {
            assert_eq!(result.expect("icon bytes"), input);
        }
    }
}
