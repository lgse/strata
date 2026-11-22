// SPDX-License-Identifier: MIT

use std::{
    fs,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

use super::*;
use crate::{
    settings::{
        FinishedSave, PendingSave, Preferences, RunePreferences, SaveOutcome, load_rune,
        parse_preferences, settings_path,
    },
    storage::writer,
};

fn finish_save(save: PendingSave) -> FinishedSave {
    writer::settle();
    let mut future = pin!(save.finished());
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(finished) => finished,
        Poll::Pending => panic!("the writer settled and the save has no result"),
    }
}

#[test]
fn defaults_leave_transparency_disabled_and_sensible_subvalues() {
    let defaults = TransparencyPreferences::default();
    assert!(!defaults.enabled);
    assert_eq!(
        defaults.background_transparency,
        DEFAULT_BACKGROUND_TRANSPARENCY
    );
    assert_eq!(
        defaults.raised_surface_transparency,
        DEFAULT_RAISED_SURFACE_TRANSPARENCY
    );
    assert!(defaults.frosted_glass);
    assert_eq!(
        defaults.unfocused_multiplier,
        DEFAULT_UNFOCUSED_TRANSPARENCY_MULTIPLIER
    );
    assert!(!defaults.varies_with_focus());
    assert!(
        TransparencyPreferences {
            enabled: true,
            ..defaults
        }
        .varies_with_focus()
    );
    assert!(
        !TransparencyPreferences {
            enabled: true,
            unfocused_multiplier: 1.0,
            ..defaults
        }
        .varies_with_focus()
    );
}

#[test]
fn clamping_and_validation_handle_out_of_range_and_non_finite_values() {
    assert_eq!(normalized_surface_transparency(0), 0);
    assert_eq!(normalized_surface_transparency(42), 42);
    assert_eq!(normalized_surface_transparency(100), 100);
    assert_eq!(normalized_surface_transparency(180), 100);

    assert_eq!(normalized_unfocused_transparency_multiplier(-0.5), 0.0);
    assert_eq!(normalized_unfocused_transparency_multiplier(0.0), 0.0);
    assert_eq!(normalized_unfocused_transparency_multiplier(1.256), 1.26);
    assert_eq!(normalized_unfocused_transparency_multiplier(3.5), 2.0);
    assert_eq!(
        normalized_unfocused_transparency_multiplier(f64::NAN),
        DEFAULT_UNFOCUSED_TRANSPARENCY_MULTIPLIER
    );
    assert_eq!(
        normalized_unfocused_transparency_multiplier(f64::INFINITY),
        DEFAULT_UNFOCUSED_TRANSPARENCY_MULTIPLIER
    );

    let custom = TransparencyPreferences {
        enabled: true,
        background_transparency: 250,
        raised_surface_transparency: 101,
        frosted_glass: false,
        unfocused_multiplier: 5.0,
    }
    .normalized();
    assert_eq!(custom.background_transparency, 100);
    assert_eq!(custom.raised_surface_transparency, 100);
    assert_eq!(custom.unfocused_multiplier, 2.0);
}

#[test]
fn legacy_settings_toml_and_json_missing_transparency_fields_deserialize_to_defaults() {
    let legacy_toml = "mode = \"theme\"\ntheme = \"tokyo-night\"\nelement_glow = true\n";
    let prefs = parse_preferences(legacy_toml).expect("parse legacy preferences");
    assert!(prefs.element_glow);

    let (parsed, repaired) = parse_transparency_preferences(legacy_toml);
    assert_eq!(parsed, TransparencyPreferences::default());
    assert!(repaired.is_empty());

    let from_empty_json: TransparencyPreferences =
        serde_json::from_str("{}").expect("deserialize empty JSON");
    assert_eq!(from_empty_json, TransparencyPreferences::default());
}

#[test]
fn toml_and_json_round_trip_preserve_all_five_fields() {
    let original = TransparencyPreferences {
        enabled: true,
        background_transparency: 35,
        raised_surface_transparency: 20,
        frosted_glass: false,
        unfocused_multiplier: 1.45,
    };

    let json = serde_json::to_string(&original).expect("serialize JSON");
    let decoded_json: TransparencyPreferences =
        serde_json::from_str(&json).expect("deserialize JSON");
    assert_eq!(decoded_json, original);

    let mut table = toml::Table::new();
    original.insert_into_table(&mut table);
    let encoded_toml = toml::to_string(&table).expect("serialize TOML");
    let (decoded_toml, repaired) = parse_transparency_preferences(&encoded_toml);
    assert_eq!(decoded_toml, original);
    assert!(repaired.is_empty());
}

#[test]
fn invalid_or_out_of_range_toml_entries_salvage_individually() {
    let broken = "\
        transparency_enabled = \"yes\"\n\
        background_transparency = 140\n\
        raised_surface_transparency = -5\n\
        frosted_glass = false\n\
        unfocused_transparency_multiplier = 4.2\n";
    let (parsed, repaired) = parse_transparency_preferences(broken);
    assert!(!parsed.enabled);
    assert_eq!(parsed.background_transparency, 100);
    assert_eq!(parsed.raised_surface_transparency, 0);
    assert!(!parsed.frosted_glass);
    assert_eq!(parsed.unfocused_multiplier, 2.0);
    assert!(repaired.contains(&TRANSPARENCY_ENABLED_KEY));
    assert!(repaired.contains(&BACKGROUND_TRANSPARENCY_KEY));
    assert!(repaired.contains(&RAISED_SURFACE_TRANSPARENCY_KEY));
    assert!(repaired.contains(&UNFOCUSED_TRANSPARENCY_MULTIPLIER_KEY));
    assert!(!repaired.contains(&FROSTED_GLASS_KEY));
}

#[test]
fn load_and_commit_rune_persists_transparency_without_clobbering_other_keys() {
    crate::test_support::in_own_process(
        "settings::transparency::tests::load_and_commit_rune_persists_transparency_without_clobbering_other_keys",
        || {
            let path = settings_path();
            fs::create_dir_all(path.parent().expect("settings parent")).expect("config dir");
            // Legacy removed keys like `tenxer_fx` load cleanly without error or clobbering.
            fs::write(
                &path,
                "mode = \"theme\"\ntheme = \"nord\"\ntenxer_fx = \"plasma_splash\"\n",
            )
            .expect("seed settings");

            let (prefs, rune, mut saving) = load_rune();
            assert!(saving.load_failure().is_none());
            assert_eq!(prefs.theme, "nord");
            assert_eq!(rune.transparency, TransparencyPreferences::default());

            let next = RunePreferences {
                transparency: TransparencyPreferences {
                    enabled: true,
                    background_transparency: 28,
                    raised_surface_transparency: 12,
                    frosted_glass: false,
                    unfocused_multiplier: 1.5,
                },
            };
            let commit = saving.commit_rune(&prefs, next);
            assert!(commit.changed);
            let finished = finish_save(commit.save.expect("save queued"));
            assert!(matches!(saving.saved(finished), SaveOutcome::Saved));

            let saved_text = fs::read_to_string(&path).expect("read saved settings");
            let (reloaded, repaired) = parse_transparency_preferences(&saved_text);
            assert_eq!(reloaded, next.transparency);
            assert!(repaired.is_empty());

            let reloaded_prefs: Preferences =
                parse_preferences(&saved_text).expect("parse preferences");
            assert_eq!(reloaded_prefs.theme, "nord");
        },
    );
}
