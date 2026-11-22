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
        FinishedSave, PendingSave, Preferences, SaveOutcome, load_with_tenxer_fx,
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
fn all_variants_round_trip_through_toml() {
    for effect in TenxerFx::ALL {
        let encoded =
            toml::to_string(&StoredTenxerFx { tenxer_fx: effect }).expect("serialize tenxer_fx");
        assert!(
            encoded.contains(effect.as_str()),
            "{encoded:?} contains {}",
            effect.as_str()
        );
        let (parsed, repaired) = parse_tenxer_fx(&encoded);
        assert_eq!(parsed, effect);
        assert!(!repaired);
    }
}

#[test]
fn legacy_settings_toml_without_tenxer_fx_defaults_to_horizon_scan() {
    let legacy = "mode = \"theme\"\ntheme = \"tokyo-night\"\ntenxer_mode = true\n";
    let prefs = parse_preferences(legacy).expect("parse legacy preferences");
    assert!(prefs.tenxer_mode);

    let (fx, repaired) = parse_tenxer_fx(legacy);
    assert_eq!(fx, TenxerFx::HorizonScan);
    assert!(!repaired);
}

#[test]
fn invalid_tenxer_fx_value_salvages_to_default_and_marks_repaired() {
    for broken in [
        "tenxer_fx = \"unknown_concept\"\n",
        "tenxer_fx = 42\n",
        "tenxer_fx = true\n",
    ] {
        let (fx, repaired) = parse_tenxer_fx(broken);
        assert_eq!(fx, TenxerFx::default(), "broken: {broken}");
        assert!(repaired, "broken: {broken}");
    }
}

#[test]
fn load_and_commit_with_tenxer_fx_persists_without_clobbering_other_keys() {
    crate::test_support::in_own_process(
        "settings::tenxer_fx::tests::load_and_commit_with_tenxer_fx_persists_without_clobbering_other_keys",
        || {
            let path = settings_path();
            fs::create_dir_all(path.parent().expect("settings parent")).expect("config dir");
            fs::write(
                &path,
                "mode = \"theme\"\ntheme = \"nord\"\ntenxer_mode = false\n",
            )
            .expect("seed legacy settings");

            let (prefs, fx, mut saving) = load_with_tenxer_fx();
            assert_eq!(prefs.theme, "nord");
            assert_eq!(fx, TenxerFx::HorizonScan);

            let commit = saving.commit_with_tenxer_fx(&prefs, TenxerFx::PerimeterIgnite);
            assert!(commit.changed);
            let finished = finish_save(commit.save.expect("save queued"));
            assert!(matches!(saving.saved(finished), SaveOutcome::Saved));

            let saved_text = fs::read_to_string(&path).expect("read saved settings");
            let (reloaded_fx, repaired) = parse_tenxer_fx(&saved_text);
            assert_eq!(reloaded_fx, TenxerFx::PerimeterIgnite);
            assert!(!repaired);

            let reloaded_prefs: Preferences =
                parse_preferences(&saved_text).expect("parse preferences");
            assert_eq!(reloaded_prefs.theme, "nord");
        },
    );
}
