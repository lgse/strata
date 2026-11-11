//! Shared by W2 (the semantic set) and K3 (the catalogs): every catalog keeps these.

use std::collections::BTreeSet;

use super::*;

fn sets() -> Vec<IconSet> {
    let mut sets = vec![PLACEHOLDER];
    sets.extend(catalogs());
    sets.push(catppuccin::LATTE);
    sets
}

#[test]
fn every_name_resolves_in_every_catalog() {
    for set in sets() {
        for &name in IconName::ALL {
            let icon = (set.icon)(name);
            assert!(
                !icon.body.is_empty() && icon.view > 0.0,
                "{}: {name:?} has an empty icon",
                set.name
            );
        }
    }
}

#[test]
fn every_icon_in_a_catalog_is_found_by_its_name() {
    for set in sets() {
        let mut names = BTreeSet::new();
        for icon in set.all {
            assert!(names.insert(icon.name), "{}: {} twice", set.name, icon.name);
            let found =
                (set.named)(icon.name).unwrap_or_else(|| panic!("{}: {}", set.name, icon.name));
            assert!(
                std::ptr::eq(found, icon),
                "{}: {} finds another icon",
                set.name,
                icon.name
            );
        }
        assert!((set.named)("no-such-icon").is_none(), "{}", set.name);
    }
}

#[test]
fn the_placeholder_draws_each_name_with_its_own_icon() {
    for &name in IconName::ALL {
        let icon = (PLACEHOLDER.icon)(name);
        assert_eq!(icon.name, name.id());
    }
    let ids: BTreeSet<_> = IconName::ALL.iter().map(|n| n.id()).collect();
    assert_eq!(ids.len(), IconName::ALL.len(), "ids are unique");
}

// K3: the generated catalogs.

/// Strata's three catalogs: Theme colors and both Brand flavours.
fn k3_sets() -> [IconSet; 3] {
    [lucide::SET, catppuccin::MOCHA, catppuccin::LATTE]
}

fn is_catppuccin(icon: &Icon) -> bool {
    icon.name.starts_with("strata-ctp-")
}

/// Rune's own rule (`rune::text::color_icon`): a body that names a colour draws in it, as a
/// colour image; any other takes the element's colour.
fn names_its_colours(icon: &Icon) -> bool {
    ["fill=\"#", "stroke=\"#", "stop-color=\"#"]
        .iter()
        .any(|p| icon.body.contains(p))
}

/// Catppuccin icons that keep a colour of their own under Theme colors, as in Strata: a white
/// clip path and a gradient stop.
const OWN_COLOURS_UNDER_THEME: [&str; 2] = ["strata-ctp-humans", "strata-ctp-poetry-lock"];

#[test]
fn a_semantic_icon_is_the_catalogs_own_icon_by_its_name() {
    for set in k3_sets() {
        for &name in IconName::ALL {
            let icon = (set.icon)(name);
            let found = (set.named)(icon.name).expect("in the catalog");
            assert!(std::ptr::eq(found, icon), "{}: {name:?}", set.name);
        }
    }
}

#[test]
fn a_semantic_id_finds_the_semantic_icon() {
    for set in k3_sets() {
        for &name in IconName::ALL {
            let found = (set.named)(name.id()).expect("an id resolves");
            assert!(
                std::ptr::eq(found, (set.icon)(name)),
                "{}: {}",
                set.name,
                name.id()
            );
        }
    }
}

#[test]
fn unknown_keys_find_nothing_so_components_fall_back() {
    for set in k3_sets() {
        for key in [
            "",
            "strata-ctp-no-such",
            "STRATA-FOLDER",
            "Folder",
            "kit-close",
        ] {
            assert!((set.named)(key).is_none(), "{}: {key:?}", set.name);
        }
    }
}

#[test]
fn file_icon_colours_leave_interface_and_lucide_icons_alone() {
    let [theme, mocha, latte] = k3_sets();
    for ((t, m), l) in theme.all.iter().zip(mocha.all).zip(latte.all) {
        assert_eq!(
            (t.name, m.name),
            (l.name, l.name),
            "the catalogs hold the same icons"
        );
        if !is_catppuccin(t) {
            assert!(t.body == m.body && m.body == l.body, "{}", t.name);
        }
    }
    for &name in IconName::ALL {
        let bodies = k3_sets().map(|set| (set.icon)(name).body);
        assert!(bodies.iter().all(|b| *b == bodies[0]), "{name:?}");
        assert!(
            !is_catppuccin((theme.icon)(name)),
            "{name:?} draws a Lucide icon"
        );
    }
}

#[test]
fn brand_icons_paint_their_own_colours_and_the_rest_take_the_tint() {
    for set in [catppuccin::MOCHA, catppuccin::LATTE] {
        for icon in set.all.iter().filter(|i| is_catppuccin(i)) {
            assert!(names_its_colours(icon), "{}: {}", set.name, icon.name);
            assert!(
                !icon.body.contains("currentColor"),
                "{}: {}",
                set.name,
                icon.name
            );
        }
    }
    for icon in lucide::SET.all {
        let own = OWN_COLOURS_UNDER_THEME.contains(&icon.name);
        assert_eq!(names_its_colours(icon), own, "{}", icon.name);
    }
}

#[test]
fn mocha_and_latte_differ_on_every_file_icon() {
    let pairs = catppuccin::MOCHA.all.iter().zip(catppuccin::LATTE.all);
    let file_icons: Vec<_> = pairs.filter(|(m, _)| is_catppuccin(m)).collect();
    assert!(!file_icons.is_empty());
    for (m, l) in file_icons {
        assert_ne!(m.body, l.body, "{}", m.name);
    }
}

#[test]
fn a_dark_background_gets_mocha_and_a_light_one_latte() {
    use crate::theme::bundled_theme;
    let flavour = |id: &str| {
        let theme = bundled_theme(id).expect("bundled");
        catppuccin::for_background(theme.dark_background()).name
    };
    assert_eq!(flavour("catppuccin-mocha"), "catppuccin-mocha");
    assert_eq!(flavour("catppuccin-latte"), "catppuccin-latte");
    assert_eq!(
        catppuccin::SET.name,
        catppuccin::MOCHA.name,
        "Mocha is the default"
    );
}

#[test]
fn every_single_colour_icon_paints() {
    for icon in lucide::SET.all.iter().filter(|i| !names_its_colours(i)) {
        let coverage = rune::text::icon_coverage(icon, 16);
        assert!(
            coverage.iter().any(|&c| c > 0),
            "{} draws nothing",
            icon.name
        );
    }
}
