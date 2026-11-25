// SPDX-License-Identifier: MIT

use std::{path::Path, time::UNIX_EPOCH};

use strata_components::{
    contract::{EntrySource, ThumbKey},
    thumbnails::State,
};
use strata_core::model::Location;
use strata_fs::adapters::thumbnail_cache;

use super::super::{Where, commit_listing_filter, commit_listing_search, flush_search, open};
use crate::{
    App,
    commands::{self, On},
    host::offscreen::Offscreen,
    shell,
    test_support::{PATIENCE, view::hosted_with},
};

const MODULE: &str = "filter::tests::thumbs";
const FILLER_COUNT: usize = 200;

fn png_bytes(rgb: [u8; 3]) -> Vec<u8> {
    let mut png = Vec::new();
    let image = image::RgbaImage::from_pixel(16, 16, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encode png");
    png
}

fn write_cached_png(path: &Path, rgb: [u8; 3]) {
    let png = png_bytes(rgb);
    std::fs::write(path, &png).expect("write png");
    let mtime = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .expect("png mtime")
        .duration_since(UNIX_EPOCH)
        .expect("mtime after epoch")
        .as_secs() as i64;
    let owned = path.to_path_buf();
    std::thread::spawn(move || thumbnail_cache::store(&owned, mtime, &png))
        .join()
        .expect("store cached thumbnail");
}

fn image_fixture() -> tempfile::TempDir {
    let folder = tempfile::tempdir().expect("folder");
    write_cached_png(&folder.path().join("a_onscreen.png"), [30, 140, 220]);
    for index in 0..FILLER_COUNT {
        std::fs::write(folder.path().join(format!("b_{index:03}.txt")), "").expect("filler");
    }
    write_cached_png(&folder.path().join("z_offscreen.png"), [220, 90, 40]);
    let sub = folder.path().join("sub");
    std::fs::create_dir(&sub).expect("sub");
    write_cached_png(&sub.join("nested_pic.png"), [50, 190, 110]);
    folder
}

fn open_folder(host: &mut Offscreen, place: Where, folder: &Path) {
    host.app().open(Location::local(folder));
    let listed = host.run_until(PATIENCE, |host| {
        place.listed(host.app(), 0)
            && place
                .listing(host.app(), 0)
                .is_some_and(|listing| listing.len() >= FILLER_COUNT + 2)
    });
    assert!(listed, "{place:?}: folder was not listed");
}

fn key_in_source(source: &dyn EntrySource, name: &str) -> Option<ThumbKey> {
    (0..source.len()).find_map(|index| {
        let entry = source.entry(index);
        (entry.name == name).then_some(entry.thumbnail).flatten()
    })
}

fn key_in_listing(app: &App, place: Where, name: &str) -> Option<ThumbKey> {
    key_in_source(place.listing(app, 0)?, name)
}

fn key_in_hits(app: &App, place: Where, name: &str) -> Option<ThumbKey> {
    if !place.shows_hits(app, 0) {
        return None;
    }
    let (source, _) = place.shown_with_selection(app, 0)?;
    key_in_source(source, name)
}

fn is_ready(app: &App, key: ThumbKey) -> bool {
    matches!(shell::thumbnails(app).state(key), State::Ready { .. })
}

fn filter_for(host: &mut Offscreen, place: Where, query: &str, expected: &str) -> ThumbKey {
    host.runtime().update(|app, tx| {
        assert!(open(app, Some(query), tx));
        flush_search(app, query);
    });
    let arrived = host.run_until(PATIENCE, |host| {
        key_in_hits(host.app(), place, expected).is_some()
    });
    assert!(
        arrived,
        "{place:?}: expected hit `{expected}` for `{query}`"
    );
    key_in_hits(host.app(), place, expected).expect("hit key")
}

fn wait_ready(host: &mut Offscreen, key: ThumbKey, context: &str) {
    let ready = host.run_until(PATIENCE, |host| is_ready(host.app(), key));
    assert!(
        ready,
        "{context}: expected State::Ready, got {:?}",
        shell::thumbnails(host.app()).state(key)
    );
}

fn assert_filtered_thumbnails_load(mode: &'static str, place: Where, name: &str) {
    let prefs = "show_thumbnails = true\nfilter_include_subfolders = true\n".to_owned();
    hosted_with(MODULE, mode, prefs, name, |mut host| {
        let folder = image_fixture();
        open_folder(&mut host, place, folder.path());

        let onscreen_key =
            key_in_listing(host.app(), place, "a_onscreen.png").expect("onscreen key");
        wait_ready(&mut host, onscreen_key, &format!("{mode} onscreen"));

        let offscreen_key =
            key_in_listing(host.app(), place, "z_offscreen.png").expect("offscreen key");
        assert_eq!(
            shell::thumbnails(host.app()).state(offscreen_key),
            State::Empty,
            "{mode}: z_offscreen.png should not have decoded while offscreen"
        );

        let hit_onscreen = filter_for(&mut host, place, "a_onscreen", "a_onscreen.png");
        assert_eq!(
            hit_onscreen, onscreen_key,
            "{mode}: onscreen filter hit should reuse the listing's thumbnail key"
        );
        assert!(
            is_ready(host.app(), hit_onscreen),
            "{mode}: onscreen filter hit should already be ready"
        );

        let hit_offscreen = filter_for(&mut host, place, "z_offscreen", "z_offscreen.png");
        wait_ready(&mut host, hit_offscreen, &format!("{mode} offscreen hit"));

        let hit_nested = filter_for(&mut host, place, "nested_pic", "nested_pic.png");
        wait_ready(&mut host, hit_nested, &format!("{mode} subfolder hit"));
    });
}

fn assert_tenxer_thumbnails_load(mode: &'static str, place: Where, name: &str) {
    let prefs = "show_thumbnails = true\ntenxer_mode = true\n".to_owned();
    hosted_with(MODULE, mode, prefs, name, |mut host| {
        let folder = image_fixture();
        open_folder(&mut host, place, folder.path());

        let onscreen_key =
            key_in_listing(host.app(), place, "a_onscreen.png").expect("onscreen key");
        wait_ready(&mut host, onscreen_key, &format!("{mode} 10xer onscreen"));

        let offscreen_key =
            key_in_listing(host.app(), place, "z_offscreen.png").expect("offscreen key");
        assert_eq!(
            shell::thumbnails(host.app()).state(offscreen_key),
            State::Empty,
            "{mode} 10xer: z_offscreen.png should start undecoded"
        );

        host.runtime().update(|app, tx| {
            commit_listing_filter(app, "z_offscreen", tx);
        });
        let arrived = host.run_until(PATIENCE, |host| {
            key_in_hits(host.app(), place, "z_offscreen.png").is_some()
        });
        assert!(arrived, "{mode} 10xer: z_offscreen.png did not arrive");
        let hit_offscreen = key_in_hits(host.app(), place, "z_offscreen.png").expect("hit key");
        wait_ready(&mut host, hit_offscreen, &format!("{mode} 10xer f filter"));

        host.runtime().update(|app, tx| {
            commit_listing_search(app, "nested_pic", tx);
        });
        let arrived = host.run_until(PATIENCE, |host| {
            key_in_hits(host.app(), place, "nested_pic.png").is_some()
        });
        assert!(arrived, "{mode} 10xer: nested_pic.png did not arrive");
        let hit_nested = key_in_hits(host.app(), place, "nested_pic.png").expect("hit key");
        wait_ready(&mut host, hit_nested, &format!("{mode} 10xer s search"));
    });
}

fn assert_show_thumbnails_toggle_while_filtered(mode: &'static str, place: Where, name: &str) {
    let prefs = "show_thumbnails = false\nfilter_include_subfolders = true\n".to_owned();
    hosted_with(MODULE, mode, prefs, name, |mut host| {
        let folder = image_fixture();
        open_folder(&mut host, place, folder.path());

        let hit_nested = filter_for(&mut host, place, "nested_pic", "nested_pic.png");
        host.runtime().frame(0.1);
        assert!(!shell::thumbnails(host.app()).enabled());
        assert_eq!(
            shell::thumbnails(host.app()).state(hit_nested),
            State::Empty,
            "{mode}: thumbnails must stay empty while show_thumbnails is off"
        );

        host.runtime().update(|app, _| {
            let _ = app.window.thumbnails_mut().set_enabled(true);
        });
        wait_ready(
            &mut host,
            hit_nested,
            &format!("{mode} enabling show_thumbnails while filtered"),
        );

        host.runtime().update(|app, _| {
            let _ = app.window.thumbnails_mut().set_enabled(false);
        });
        host.runtime().frame(0.2);
        assert!(!shell::thumbnails(host.app()).enabled());
    });
}

#[test]
fn filtered_offscreen_and_subfolder_thumbnails_load_in_columns() {
    assert_filtered_thumbnails_load(
        "columns",
        Where::Columns,
        "filtered_offscreen_and_subfolder_thumbnails_load_in_columns",
    );
}

#[test]
fn filtered_offscreen_and_subfolder_thumbnails_load_in_list() {
    assert_filtered_thumbnails_load(
        "list",
        Where::List,
        "filtered_offscreen_and_subfolder_thumbnails_load_in_list",
    );
}

#[test]
fn filtered_offscreen_and_subfolder_thumbnails_load_in_icons() {
    assert_filtered_thumbnails_load(
        "icons",
        Where::Icons,
        "filtered_offscreen_and_subfolder_thumbnails_load_in_icons",
    );
}

#[test]
fn tenxer_filter_and_search_load_thumbnails_in_columns() {
    assert_tenxer_thumbnails_load(
        "columns",
        Where::Columns,
        "tenxer_filter_and_search_load_thumbnails_in_columns",
    );
}

#[test]
fn tenxer_filter_and_search_load_thumbnails_in_list() {
    assert_tenxer_thumbnails_load(
        "list",
        Where::List,
        "tenxer_filter_and_search_load_thumbnails_in_list",
    );
}

#[test]
fn tenxer_filter_and_search_load_thumbnails_in_icons() {
    assert_tenxer_thumbnails_load(
        "icons",
        Where::Icons,
        "tenxer_filter_and_search_load_thumbnails_in_icons",
    );
}

#[test]
fn show_thumbnails_toggle_while_filtered_works_in_columns() {
    assert_show_thumbnails_toggle_while_filtered(
        "columns",
        Where::Columns,
        "show_thumbnails_toggle_while_filtered_works_in_columns",
    );
}

#[test]
fn show_thumbnails_toggle_while_filtered_works_in_list() {
    assert_show_thumbnails_toggle_while_filtered(
        "list",
        Where::List,
        "show_thumbnails_toggle_while_filtered_works_in_list",
    );
}

#[test]
fn show_thumbnails_toggle_while_filtered_works_in_icons() {
    assert_show_thumbnails_toggle_while_filtered(
        "icons",
        Where::Icons,
        "show_thumbnails_toggle_while_filtered_works_in_icons",
    );
}

#[test]
fn switching_from_columns_to_icons_while_filtered_loads_thumbnails() {
    let prefs = "show_thumbnails = false\nfilter_include_subfolders = true\n".to_owned();
    hosted_with(
        MODULE,
        "columns",
        prefs,
        "switching_from_columns_to_icons_while_filtered_loads_thumbnails",
        |mut host| {
            let folder = image_fixture();
            open_folder(&mut host, Where::Columns, folder.path());

            let hit = filter_for(&mut host, Where::Columns, "nested_pic", "nested_pic.png");
            assert_eq!(shell::thumbnails(host.app()).state(hit), State::Empty);

            host.runtime().update(|app, tx| {
                commands::run(app, "view-icons", On::Focus, tx);
            });
            let switched = host.run_until(PATIENCE, |host| {
                Where::shown(host.app()) == Some(Where::Icons)
                    && key_in_hits(host.app(), Where::Icons, "nested_pic.png").is_some()
            });
            assert!(switched, "view did not switch to Icons with active filter");

            host.runtime().update(|app, _| {
                let _ = app.window.thumbnails_mut().set_enabled(true);
            });
            let icons_hit =
                key_in_hits(host.app(), Where::Icons, "nested_pic.png").expect("icons hit key");
            wait_ready(
                &mut host,
                icons_hit,
                "switched from Columns to Icons while filtered",
            );
        },
    );
}
