use super::{IconResolver, SearchPath};
use crate::icon_theme::test_support::{Scratch, fixtures};

/// The fixture tree: a user root before the system root, then pixmaps.
fn resolver() -> IconResolver {
    let root = fixtures();
    IconResolver::new(SearchPath {
        icon_dirs: vec![root.join("user"), root.join("system")],
        pixmap_dirs: vec![root.join("pixmaps")],
    })
}

/// The file `names` resolve to in `theme`, relative to the fixture tree.
fn find(
    r: &mut IconResolver,
    theme: &str,
    names: &[&str],
    size: u32,
    scale: u32,
) -> Option<String> {
    let found = r.resolve(theme, names, size, scale)?;
    let path = found.path.strip_prefix(fixtures()).ok()?;
    Some(path.to_string_lossy().into_owned())
}

fn one(theme: &str, name: &str, size: u32, scale: u32) -> Option<String> {
    find(&mut resolver(), theme, &[name], size, scale)
}

#[test]
fn a_data_dir_that_isnt_utf8_is_still_searched() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let odd = OsString::from_vec(b"/data/\xff".to_vec());
    let mut dirs = odd.clone().into_vec();
    dirs.extend_from_slice(b":/usr/share");
    let dirs = OsString::from_vec(dirs);
    let search = SearchPath::from_vars(|name| match name {
        "HOME" => Some("/home/u".into()),
        "XDG_DATA_DIRS" => Some(dirs.clone()),
        _ => None,
    });
    let odd = std::path::PathBuf::from(odd);
    assert_eq!(
        search.icon_dirs,
        [
            "/home/u/.local/share/icons".into(),
            "/home/u/.icons".into(),
            odd.join("icons"),
            "/usr/share/icons".into(),
        ]
    );
    assert_eq!(
        search.pixmap_dirs,
        [odd.join("pixmaps"), "/usr/share/pixmaps".into()]
    );
}

#[test]
fn inherited_themes_are_searched_depth_first_then_hicolor() {
    let mut r = resolver();
    let mut at = |name| find(&mut r, "Child", &[name], 32, 1);
    assert_eq!(
        at("only-child").as_deref(),
        Some("system/Child/16x16/apps/only-child.png")
    );
    assert_eq!(
        at("only-parent").as_deref(),
        Some("system/Parent/32x32/apps/only-parent.png")
    );
    // Parent inherits Missing (not installed, skipped), then Grand.
    assert_eq!(
        at("only-grand").as_deref(),
        Some("system/Grand/48x48/apps/only-grand.png")
    );
    // Grand, three levels down, comes before hicolor, which no theme names.
    assert_eq!(
        at("order").as_deref(),
        Some("system/Grand/48x48/apps/order.png")
    );
    assert_eq!(
        at("only-hicolor").as_deref(),
        Some("system/hicolor/16x16/apps/only-hicolor.png")
    );
}

#[test]
fn a_first_parent_and_its_own_parents_come_before_the_second_parent() {
    // DepthA inherits DepthB, DepthC; DepthB inherits DepthD. Both DepthD and DepthC have
    // `deep`: depth-first finds DepthD's, breadth-first would find DepthC's.
    assert_eq!(
        one("DepthA", "deep", 48, 1).as_deref(),
        Some("system/DepthD/48x48/apps/deep.png")
    );
}

#[test]
fn a_cycle_ends_and_a_theme_without_an_index_is_skipped() {
    assert_eq!(
        one("Cycle", "only-grand", 48, 1).as_deref(),
        Some("system/Grand/48x48/apps/only-grand.png")
    );
    // NoIndex has the file but no index.theme, so only hicolor is searched.
    assert_eq!(one("NoIndex", "only-child", 16, 1), None);
    assert_eq!(
        one("NoIndex", "only-hicolor", 16, 1).as_deref(),
        Some("system/hicolor/16x16/apps/only-hicolor.png")
    );
}

#[test]
fn an_unknown_or_unsafe_theme_name_searches_hicolor_alone() {
    for theme in ["Nope", "", "..", "system/Child", "../system/Child"] {
        assert_eq!(one(theme, "only-child", 16, 1), None, "{theme:?}");
        assert!(one(theme, "only-hicolor", 16, 1).is_some(), "{theme:?}");
    }
}

#[test]
fn only_a_few_unknown_theme_names_are_kept() {
    let mut r = resolver();
    for i in 0..100 {
        assert!(find(&mut r, &format!("Nope{i}"), &["only-hicolor"], 16, 1).is_some());
    }
    // Each kept name is checked on every revalidation.
    assert!(
        r.themes.len() <= super::UNKNOWN_THEMES + 1,
        "{}",
        r.themes.len()
    );
    // The known ones stay.
    assert!(find(&mut r, "Child", &["only-child"], 16, 1).is_some());
    for i in 0..100 {
        find(&mut r, &format!("Nope{i}"), &["x"], 16, 1);
    }
    for theme in ["Child", "Parent", "Grand", "hicolor"] {
        assert!(r.themes.contains_key(theme), "{theme}");
    }
}

#[test]
fn themes_are_searched_outside_names() {
    let mut r = resolver();
    // `primary` is only in hicolor; the theme's own `fallback` wins over it.
    assert_eq!(
        find(&mut r, "Child", &["primary", "fallback"], 48, 1).as_deref(),
        Some("system/Child/48x48/apps/fallback.png")
    );
    assert_eq!(
        find(&mut r, "hicolor", &["primary", "fallback"], 48, 1).as_deref(),
        Some("system/hicolor/48x48/apps/primary.png")
    );
}

#[test]
fn the_closest_size_prefers_an_icon_to_scale_down() {
    let steps = |size| one("Parent", "steps", size, 1);
    assert_eq!(
        steps(32).as_deref(),
        Some("system/Parent/32x32/apps/steps.png")
    );
    assert_eq!(
        steps(64).as_deref(),
        Some("system/Parent/64x64/apps/steps.png")
    );
    // 40: 64 is farther than 32, but scales down.
    assert_eq!(
        steps(40).as_deref(),
        Some("system/Parent/64x64/apps/steps.png")
    );
    // Both larger, or both smaller: the closest.
    assert_eq!(
        steps(20).as_deref(),
        Some("system/Parent/32x32/apps/steps.png")
    );
    assert_eq!(
        steps(70).as_deref(),
        Some("system/Parent/64x64/apps/steps.png")
    );
    // 100: 256x256 (listed first) is farther than 32x32@2 (64 pixels), but scales down.
    assert_eq!(
        one("Parent", "uphill", 100, 1).as_deref(),
        Some("system/Parent/256x256/apps/uphill.png")
    );
}

#[test]
fn a_threshold_directory_covers_two_pixels_either_side() {
    let near = |size| one("Child", "near", size, 1);
    // 48 Threshold 2 covers 46..=50 exactly, beating 51 Fixed (listed before it).
    assert_eq!(
        near(50).as_deref(),
        Some("system/Child/48x48/apps/near.png")
    );
    assert_eq!(
        near(51).as_deref(),
        Some("system/Child/51x51/apps/near.png")
    );
    // 46 is inside 48's window, so 48 beats 47 Fixed, which is closer but not exact.
    assert_eq!(
        one("Child", "low", 46, 1).as_deref(),
        Some("system/Child/48x48/apps/low.png")
    );
}

#[test]
fn a_full_tie_goes_to_the_later_directory_as_in_gtk() {
    // 48 Threshold and 48 Fixed are both exact at 48, at scale 1, both bitmaps.
    assert_eq!(
        one("Child", "tie", 48, 1).as_deref(),
        Some("system/Child/extra48/apps/tie.png")
    );
}

#[test]
fn a_larger_directory_listed_after_an_exact_match_beats_it_as_in_gtk() {
    // GTK checks "prefer scaling down" before whether the best so far is exact, so 52 Fixed,
    // listed after 48 Threshold, wins at 50 although 48 covers 50 exactly.
    assert_eq!(
        one("Child", "quirk", 50, 1).as_deref(),
        Some("system/Child/52x52/apps/quirk.png")
    );
    assert_eq!(
        one("Child", "quirk", 48, 1).as_deref(),
        Some("system/Child/48x48/apps/quirk.png")
    );
}

#[test]
fn a_scalable_directory_is_the_fallback_and_loses_a_tie_to_a_bitmap() {
    let sizes = |size| one("Parent", "sizes", size, 1);
    // 48 is in the scalable range only.
    assert_eq!(
        sizes(48).as_deref(),
        Some("system/Parent/scalable/apps/sizes.svg")
    );
    // 32 is exact both as a bitmap and in the scalable range: the bitmap wins.
    assert_eq!(
        sizes(32).as_deref(),
        Some("system/Parent/32x32/apps/sizes.png")
    );
    // 12 is 4 below both 16 Fixed and the scalable MinSize 16: the bitmap wins the tie.
    assert_eq!(
        one("Parent", "small", 12, 1).as_deref(),
        Some("system/Parent/16x16/apps/small.png")
    );
    // Only a scalable file: at any size, even beyond its MaxSize.
    assert_eq!(
        one("Parent", "vector", 22, 1).as_deref(),
        Some("system/Parent/scalable/apps/vector.svg")
    );
    assert_eq!(
        one("Parent", "vector", 300, 1).as_deref(),
        Some("system/Parent/scalable/apps/vector.svg")
    );
}

#[test]
fn a_scalable_max_size_is_compared_unscaled_as_gtk_does() {
    // 100 at scale 2: GTK takes the scalable directory (MaxSize 128) as exact, where the spec
    // would give the 256 bitmap.
    assert_eq!(
        one("Parent", "big", 100, 2).as_deref(),
        Some("system/Parent/scalable/apps/big.svg")
    );
    assert_eq!(
        one("Parent", "big", 200, 1).as_deref(),
        Some("system/Parent/256x256/apps/big.png")
    );
}

#[test]
fn scale_two_directories_match_scale_two_requests() {
    let hidpi = |size, scale| one("Parent", "hidpi", size, scale);
    // 32 at 2 is 64 device pixels, exact in both 32x32@2 and 64x64: the scale decides.
    assert_eq!(
        hidpi(32, 2).as_deref(),
        Some("system/Parent/32x32@2/apps/hidpi.png")
    );
    assert_eq!(
        hidpi(64, 1).as_deref(),
        Some("system/Parent/64x64/apps/hidpi.png")
    );
    assert_eq!(
        hidpi(32, 1).as_deref(),
        Some("system/Parent/32x32/apps/hidpi.png")
    );
}

#[test]
fn a_png_beats_an_svg_and_an_xpm_is_skipped() {
    assert_eq!(
        one("Parent", "both", 32, 1).as_deref(),
        Some("system/Parent/32x32/apps/both.png")
    );
    // old.xpm sits in the exact directory, old.svg in 64x64.
    assert_eq!(
        one("Parent", "old", 32, 1).as_deref(),
        Some("system/Parent/64x64/apps/old.svg")
    );
    // GTK's .symbolic.png encoding never matches a name.
    assert_eq!(one("Parent", "odd.symbolic", 32, 1), None);
    assert_eq!(one("Parent", "odd", 32, 1), None);
}

#[test]
fn a_user_root_overrides_the_system_root() {
    assert_eq!(
        one("hicolor", "shared", 48, 1).as_deref(),
        Some("user/hicolor/48x48/apps/shared.png")
    );
}

#[test]
fn unthemed_icons_come_after_every_theme() {
    let mut r = resolver();
    let found = r.resolve("Child", &["toplevel"], 48, 1);
    assert_eq!(found.as_ref().and_then(|f| f.theme.as_deref()), None);
    assert_eq!(
        find(&mut r, "Child", &["toplevel"], 48, 1).as_deref(),
        Some("system/toplevel.png")
    );
    assert_eq!(
        find(&mut r, "Child", &["legacy"], 48, 1).as_deref(),
        Some("pixmaps/legacy.png")
    );
    assert_eq!(
        find(&mut r, "Child", &["vector-only"], 48, 1).as_deref(),
        Some("pixmaps/vector-only.svg")
    );
    assert_eq!(find(&mut r, "Child", &["xpm-only"], 48, 1), None);
    // `shared` is also a file in the system root; the theme's copy wins.
    assert_eq!(
        find(&mut r, "Child", &["shared"], 48, 1).as_deref(),
        Some("user/hicolor/48x48/apps/shared.png")
    );
    // A later name in a theme still beats an earlier unthemed one.
    assert_eq!(
        find(&mut r, "Child", &["toplevel", "only-child"], 16, 1).as_deref(),
        Some("system/Child/16x16/apps/only-child.png")
    );
}

#[test]
fn leading_symbolic_names_are_searched_through_the_whole_chain_first() {
    let mut r = resolver();
    // Child has a full-colour `app`; Parent's `app-symbolic` wins for a symbolic request.
    let found = r.resolve("Child", &["app-symbolic", "app"], 48, 1);
    assert!(found.as_ref().is_some_and(super::Resolved::is_symbolic));
    assert_eq!(
        find(&mut r, "Child", &["app-symbolic", "app"], 48, 1).as_deref(),
        Some("system/Parent/scalable/apps/app-symbolic.svg")
    );
    // A request that leads with the full-colour name gets Child's.
    assert_eq!(
        find(&mut r, "Child", &["app", "app-symbolic"], 48, 1).as_deref(),
        Some("system/Child/48x48/apps/app.png")
    );
}

#[test]
fn a_missing_icon_is_none_and_names_are_never_paths() {
    let mut r = resolver();
    assert_eq!(
        find(&mut r, "Child", &["nothing", "nothing-symbolic"], 48, 1),
        None
    );
    assert_eq!(
        find(
            &mut r,
            "Child",
            &["../toplevel", "48x48/apps/app", ""],
            48,
            1
        ),
        None
    );
    // A directory whose Size isn't a number is left out.
    assert_eq!(find(&mut r, "Child", &["unlisted"], 48, 1), None);
}

#[test]
fn a_hicolor_without_an_index_is_derived_from_its_directories() {
    let root = fixtures().join("derived");
    let mut r = IconResolver::new(SearchPath {
        icon_dirs: vec![root],
        pixmap_dirs: Vec::new(),
    });
    let mut at = |name, size, scale| {
        r.resolve("Anything", &[name], size, scale).and_then(|f| {
            Some(
                f.path
                    .strip_prefix(fixtures())
                    .ok()?
                    .to_string_lossy()
                    .into_owned(),
            )
        })
    };
    assert_eq!(
        at("derived", 48, 1).as_deref(),
        Some("derived/hicolor/48x48/apps/derived.png")
    );
    assert_eq!(
        at("derived", 128, 1).as_deref(),
        Some("derived/hicolor/scalable/apps/derived.svg")
    );
    assert_eq!(
        at("hidpi-derived", 16, 2).as_deref(),
        Some("derived/hicolor/16x16@2/apps/hidpi-derived.png")
    );
}

#[test]
fn revalidate_reads_again_only_after_a_change() {
    let scratch = Scratch::new("resolver");
    scratch.put(
        "T/index.theme",
        "[Icon Theme]\nDirectories=48x48/apps\n[48x48/apps]\nSize=48\n",
    );
    scratch.put("T/48x48/apps/first.png", "");
    let mut r = IconResolver::new(SearchPath {
        icon_dirs: vec![scratch.root().to_path_buf()],
        pixmap_dirs: Vec::new(),
    });
    assert!(r.resolve("T", &["first"], 48, 1).is_some());
    assert!(r.resolve("T", &["late"], 48, 1).is_none());
    assert!(r.resolve("T", &["root-late"], 48, 1).is_none());
    assert!(!r.revalidate());
    // An icon added to a directory already read, and one to a root, are found after the check.
    scratch.put("T/48x48/apps/late.png", "");
    scratch.put("root-late.svg", "");
    assert!(r.revalidate());
    assert!(r.resolve("T", &["late"], 48, 1).is_some());
    assert!(r.resolve("T", &["root-late"], 48, 1).is_some());
    assert!(!r.revalidate());
    // A theme installed after a lookup asked for it.
    assert!(r.resolve("Later", &["later"], 48, 1).is_none());
    scratch.put(
        "Later/index.theme",
        "[Icon Theme]\nDirectories=48x48/apps\n[48x48/apps]\nSize=48\n",
    );
    scratch.put("Later/48x48/apps/later.png", "");
    assert!(r.revalidate());
    assert!(r.resolve("Later", &["later"], 48, 1).is_some());
}
