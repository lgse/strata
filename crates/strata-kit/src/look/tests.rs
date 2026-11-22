// SPDX-License-Identifier: MIT

use super::*;

fn ladder() -> Ladder {
    Ladder::new(vec![32.0, 48.0, 64.0, 96.0], 64.0).expect("valid")
}

#[test]
fn a_step_moves_along_the_ladder_and_stops_at_its_ends() {
    let l = ladder();
    assert_eq!(l.step(64.0, 1), 96.0);
    assert_eq!(l.step(64.0, -1), 48.0);
    assert_eq!(l.step(48.0, 2), 96.0);
    assert_eq!(l.step(96.0, 1), 96.0);
    assert_eq!(l.step(32.0, -3), 32.0);
}

#[test]
fn a_step_from_between_steps_lands_on_the_next_one_in_its_direction() {
    let l = ladder();
    assert_eq!(l.step(50.0, 1), 64.0);
    assert_eq!(l.step(50.0, -1), 48.0);
    assert_eq!(l.step(20.0, 1), 32.0);
    assert_eq!(l.step(300.0, -1), 96.0);
    assert_eq!(l.step(50.0, 0), 50.0);
}

#[test]
fn a_ladder_must_increase_and_hold_its_default() {
    assert!(Ladder::new(vec![], 0.0).is_none());
    assert!(Ladder::new(vec![32.0, 32.0], 32.0).is_none());
    assert!(Ladder::new(vec![64.0, 32.0], 32.0).is_none());
    assert!(Ladder::new(vec![32.0, 64.0], 48.0).is_none());
    assert!(Ladder::new(vec![32.0, f32::NAN], 32.0).is_none());
    assert!(Ladder::new(vec![0.0, 32.0], 32.0).is_none());
    assert!(Ladder::new(vec![-8.0, 32.0], 32.0).is_none());
}

#[test]
fn a_tile_takes_the_smallest_thumbnail_class_that_covers_it() {
    let t = ThumbnailSteps::new(vec![128.0, 256.0]).expect("valid");
    assert_eq!(t.for_size(32.0), 0);
    assert_eq!(t.for_size(128.0), 0);
    assert_eq!(t.for_size(129.0), 1);
    assert_eq!(t.for_size(1024.0), 1);
    assert!(ThumbnailSteps::new(vec![]).is_none());
    assert!(ThumbnailSteps::new(vec![0.0, 128.0]).is_none());
}

/// Rune draws streamed thumbnails with no mipmaps, so a class more than twice its draw
/// aliases: every tile on Strata's ladder, and a 16 pt list icon, at the usual scales.
#[test]
fn strata_thumbnails_are_never_held_at_more_than_twice_their_drawn_size() {
    let look = Look::fallback(true);
    let classes = &look.thumbnails;
    let largest = *classes.sizes().last().expect("a class");
    let sizes = std::iter::once(16.0).chain(look.tiles.steps().iter().copied());
    for points in sizes {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let px = points * scale;
            let class = classes.sizes()[classes.for_size(px)];
            if px <= largest {
                assert!(
                    class >= px && class <= 2.0 * px,
                    "{points} pt at {scale}: {class}"
                );
            } else {
                assert_eq!(class, largest, "{points} pt at {scale}");
            }
        }
    }
}

#[test]
fn an_icon_key_finds_the_catalog_icon_of_that_name() {
    let look = Look::fallback(false);
    let folder = look.icon(IconName::Folder);
    let found = look.icon_named(folder.name).expect("in the catalog");
    assert!(std::ptr::eq(found, folder));
    assert!(look.icon_named("no-such-icon").is_none());
}

fn hash(x: &impl std::hash::Hash) -> u64 {
    use std::hash::BuildHasher;
    std::hash::BuildHasherDefault::<std::hash::DefaultHasher>::default().hash_one(x)
}

/// A change to one token struct.
type Change = fn(&mut Look);

/// A memo keyed on the token structs reruns when a token changes, and only then.
#[test]
fn token_structs_hash_as_memo_versions() {
    let a = Look::fallback(false);
    let key = |l: &Look| {
        hash(&(
            (
                l.space,
                l.frame,
                l.content,
                l.row,
                l.icon_size,
                l.glow,
                l.ty,
            ),
            (&l.tiles, &l.thumbnails, l.syntax_extra, l.fill),
        ))
    };
    assert_eq!(key(&a), key(&Look::fallback(false)));

    let changes: [(&str, Change); 11] = [
        ("space", |l| l.space.xs += 1.0),
        ("frame", |l| l.frame.inset += 1.0),
        ("content", |l| l.content.pad += 1.0),
        ("row", |l| l.row.list += 1.0),
        ("icon_size", |l| l.icon_size.md += 1.0),
        ("glow", |l| l.glow.opacity /= 2.0),
        ("ty", |l| l.ty.title.size += 1.0),
        ("tiles", |l| {
            l.tiles = Ladder::new(vec![32.0, 64.0], 64.0).expect("valid")
        }),
        ("thumbnails", |l| {
            l.thumbnails = ThumbnailSteps::new(vec![256.0]).expect("valid")
        }),
        ("syntax_extra", |l| {
            l.syntax_extra.preprocessor = l.kit.c.danger.fg
        }),
        ("fill", |l| l.fill.highlight = l.kit.c.danger.fg),
    ];
    for (name, change) in changes {
        let mut b = a.clone();
        change(&mut b);
        assert_ne!(key(&a), key(&b), "a changed {name} hashes differently");
    }

    let (mut zero, mut negative) = (a.space, a.space);
    zero.xxs = 0.0;
    negative.xxs = -0.0;
    assert_eq!(hash(&zero), hash(&negative), "-0.0 hashes as 0.0");
}

#[test]
fn the_fallback_follows_the_appearance() {
    for dark in [false, true] {
        let look = Look::fallback(dark);
        assert_eq!(look.kit.dark, dark);
        assert_eq!(fallback(dark).c.bg_base, look.kit.c.bg_base);
    }
    assert_ne!(
        Look::fallback(true).kit.c.bg_base,
        Look::fallback(false).kit.c.bg_base
    );
}

/// A radius Kit has no token for rounds as Kit's own do, so the two agree at every roundness.
#[test]
fn a_rounded_radius_matches_kit_at_the_same_base() {
    let base = crate::shape::BASE_RADIUS;
    for roundness in [0, 30, 60, 70, 130, 200] {
        let look = Look::fallback(false).with_params(&LookParams {
            roundness,
            ..LookParams::default()
        });
        let r = look.kit.radius;
        let pairs = [
            (base.xs, r.xs),
            (base.sm, r.sm),
            (base.md, r.md),
            (base.lg, r.lg),
            (base.xl, r.xl),
        ];
        for (base, kit) in pairs {
            assert_eq!(look.round(base), kit, "{base} at {roundness} %");
        }
    }
}

#[test]
fn accent_icon_kit_routes_icon_button_roles_to_accent() {
    let look = Look::fallback(true);
    let accent = look.kit.c.icon_accent;
    let ik = look.accent_icon_kit();
    assert_eq!(ik.c.text_base, accent);
    assert_eq!(ik.c.icon_base, accent);
    assert_eq!(ik.c.icon_muted, accent);
    assert_eq!(ik.c.bg_base, look.kit.c.bg_base);
    assert_eq!(ik.c.hover, look.kit.c.hover);
    assert_eq!(ik.c.pressed, look.kit.c.pressed);
    assert_eq!(ik.c.border_focus, look.kit.c.border_focus);
}

#[test]
fn help_look_matches_gtk_tokens_across_themes_roundness_and_text_size() {
    let dark = crate::theme::bundled_theme(crate::theme::DEFAULT_ID).expect("bundled dark theme");
    let light = crate::theme::bundled()
        .iter()
        .find(|theme| !theme.dark_background())
        .expect("bundled light theme");

    for (label, base) in [
        ("dark theme", dark.look()),
        ("light theme", light.look()),
        ("dark fallback", Look::fallback(true)),
        ("light fallback", Look::fallback(false)),
    ] {
        for (roundness, expected_radius) in [(0, 0.0), (60, 3.0), (100, 5.0), (200, 10.0)] {
            let look = base.clone().with_params(&LookParams {
                roundness,
                ..LookParams::default()
            });
            assert_eq!(
                look.kit.radius.sm, expected_radius,
                "{label} at {roundness}%"
            );
            let kit = &look.kit;
            let expected = HelpLook::system(true)
                .fill(kit.alpha(kit.c.black, 0.8))
                .ink(kit.c.white)
                .ring(1.0, kit.light_on(0.1, kit.c.black))
                .radius(expected_radius)
                .padding(10.0, 6.0)
                .font(kit.ty.ui_compact);
            assert_eq!(look.help_look(), expected, "{label} at {roundness}%");
        }
    }

    let larger = dark.look().with_params(&LookParams {
        text_size: 18,
        ..LookParams::default()
    });
    assert_ne!(
        larger.help_look(),
        dark.look().with_params(&LookParams::default()).help_look(),
        "tooltip type follows the interface text size"
    );

    let mut overridden = dark.look().with_params(&LookParams::default());
    overridden.kit.radius.sm = 7.0;
    let kit = &overridden.kit;
    assert_eq!(
        overridden.help_look(),
        HelpLook::system(true)
            .fill(kit.alpha(kit.c.black, 0.8))
            .ink(kit.c.white)
            .ring(1.0, kit.light_on(0.1, kit.c.black))
            .radius(7.0)
            .padding(10.0, 6.0)
            .font(kit.ty.ui_compact),
        "tooltip radius follows kit.radius.sm overrides"
    );
}
