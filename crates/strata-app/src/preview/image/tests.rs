// SPDX-License-Identifier: MIT

use std::{ffi::OsString, io::Cursor};

use image::{ImageFormat, Rgba, RgbaImage};
use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime};
use strata_components::viewers::{image::ImageView, viewport::Mode};
use strata_core::model::{EntryKind, FileEntry, Location, MetadataValue};
use strata_kit::look::Look;

use super::{IMAGE_KEY, ImageViewer};

const DT: f64 = 1.0 / 60.0;

struct Host {
    viewer: ImageViewer,
    look: Look,
}

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let image = RgbaImage::from_pixel(width, height, Rgba([40, 120, 200, 255]));
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("encode png");
    bytes
}

fn image_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/tmp/{name}")),
        thumbnail_path: None,
        native_name: OsString::from(name),
        display_name: name.to_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        created_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        listable: MetadataValue::Unknown,
        is_hidden: false,
        recent_uri: None,
    }
}

fn mount_viewer(width: u32, height: u32, window: (f32, f32)) -> Runtime<Host> {
    let viewer = ImageViewer::from_png(&image_entry("sample.png"), &png_bytes(width, height), None)
        .expect("png");
    let host = Host {
        viewer,
        look: Look::fallback(true),
    };
    let mut rt = Runtime::new(
        host,
        |s: &Host, cx: &mut Cx<Host>| -> El {
            ImageView::new(&s.viewer.viewport, |s: &mut Host| &mut s.viewer.viewport)
                .key(IMAGE_KEY)
                .pixels(&s.viewer.pixels)
                .name(s.viewer.name.clone())
                .size_px(s.viewer.size_px)
                .build(&s.look, cx)
        },
        window,
        Env::new(false, 1.0),
    );
    rt.frame(DT);
    rt.frame(DT);
    rt
}

#[test]
fn small_image_caps_at_one_to_one_allows_manual_zoom_and_resets_to_scale_down() {
    let mut rt = mount_viewer(128, 128, (800.0, 600.0));
    assert_eq!(rt.state.viewer.viewport.default_mode(), Mode::ScaleDown);
    assert_eq!(rt.state.viewer.viewport.mode(), Mode::ScaleDown);
    let initial = rt
        .state
        .viewer
        .viewport
        .level()
        .expect("laid-out image level");
    assert!(
        (initial - 1.0).abs() < 1e-4,
        "small image caps at 100%, got {initial}"
    );

    // Manual zoom in pushes past 100% into Mode::Free.
    rt.update(|_, tx| tx.focus(format!("{IMAGE_KEY}-picture"), true));
    rt.frame(DT);
    rt.key(KeyEvent {
        key: Key::Text("+".into()),
        mods: Mods::default(),
    });
    rt.frame(DT);
    assert_eq!(rt.state.viewer.viewport.mode(), Mode::Free);
    let zoomed_in = rt.state.viewer.viewport.level().expect("zoomed-in level");
    assert!(
        zoomed_in > 1.0,
        "manual zoom in exceeds 100%, got {zoomed_in}"
    );

    // Pressing 0 resets back to Mode::ScaleDown (100%).
    rt.key(KeyEvent {
        key: Key::Text("0".into()),
        mods: Mods::default(),
    });
    rt.frame(DT);
    assert_eq!(rt.state.viewer.viewport.mode(), Mode::ScaleDown);
    let after_key_reset = rt.state.viewer.viewport.level().expect("key reset level");
    assert!(
        (after_key_reset - 1.0).abs() < 1e-4,
        "key reset returns to 100%, got {after_key_reset}"
    );

    // Zooming out below 100% and calling ImageViewer::reset() also restores Mode::ScaleDown.
    rt.state.viewer.viewport.zoom_step(-1);
    assert_eq!(rt.state.viewer.viewport.mode(), Mode::Free);
    assert!(rt.state.viewer.viewport.level().expect("zoomed out") < 1.0);
    rt.state.viewer.reset();
    assert_eq!(rt.state.viewer.viewport.mode(), Mode::ScaleDown);
    let after_reset = rt.state.viewer.viewport.level().expect("reset level");
    assert!(
        (after_reset - 1.0).abs() < 1e-4,
        "ImageViewer::reset returns to 100%, got {after_reset}"
    );
}

#[test]
fn oversized_image_scales_down_to_fit_and_resets_below_one_to_one() {
    let mut rt = mount_viewer(2000, 1500, (600.0, 450.0));
    assert_eq!(rt.state.viewer.viewport.default_mode(), Mode::ScaleDown);
    assert_eq!(rt.state.viewer.viewport.mode(), Mode::ScaleDown);
    let initial = rt
        .state
        .viewer
        .viewport
        .level()
        .expect("oversized image level");
    assert!(
        initial < 1.0,
        "oversized image scales down below 100%, got {initial}"
    );

    rt.state.viewer.viewport.zoom_to(2.0, None);
    assert_eq!(rt.state.viewer.viewport.mode(), Mode::Free);
    assert!(rt.state.viewer.viewport.level().expect("zoomed") > 1.0);

    rt.state.viewer.reset();
    assert_eq!(rt.state.viewer.viewport.mode(), Mode::ScaleDown);
    let reset = rt.state.viewer.viewport.level().expect("reset level");
    assert!(
        (reset - initial).abs() < 1e-4 && reset < 1.0,
        "reset restores scaled-down level {initial}, got {reset}"
    );
}
