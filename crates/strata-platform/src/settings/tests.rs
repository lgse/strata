//! The portal's replies and changes, applied without a bus. The watcher's connection, its
//! threading and the portal's absence are checked in `tests/session/p5.rs`.

use std::collections::HashMap;
use std::time::Duration;

use zbus::{OwnedVariant, Variant};

use super::fallback::Reported;
use super::parse::{Reply, apply_change, apply_reply};
use super::{DesktopSettings, Fallback, Key, Setting};

fn owned(value: Variant<'static>) -> OwnedVariant {
    value.try_into_owned().expect("a plain value")
}

/// A reply holding every key, each away from its default.
fn full_reply() -> Reply {
    let mut reply = Reply::new();
    let mut put = |key: Key, value: Variant<'static>| {
        reply
            .entry(key.namespace().to_owned())
            .or_default()
            .insert(key.name().to_owned(), owned(value));
    };
    put(Key::TextScale, Variant::F64(1.1818));
    put(Key::DoubleClick, Variant::I32(1200));
    put(Key::DragThreshold, Variant::I32(12));
    put(Key::CursorBlink, Variant::Bool(false));
    put(Key::CursorBlinkTime, Variant::I32(900));
    put(Key::CursorBlinkTimeout, Variant::I32(4));
    put(Key::EnableAnimations, Variant::Bool(false));
    put(Key::ReducedMotion, Variant::U32(1));
    put(Key::RememberRecentFiles, Variant::Bool(false));
    // Keys the module doesn't read ride along in real replies.
    reply
        .entry("org.gnome.desktop.interface".into())
        .or_default()
        .insert("font-name".into(), owned(Variant::from("Sans 11")));
    reply
        .entry("org.freedesktop.appearance".into())
        .or_default()
        .insert("color-scheme".into(), owned(Variant::U32(1)));
    reply
}

fn reply_with(key: Key, value: Variant<'static>) -> Reply {
    HashMap::from([(
        key.namespace().to_owned(),
        HashMap::from([(key.name().to_owned(), owned(value))]),
    )])
}

#[test]
fn a_full_reply_sets_every_key_and_marks_it_known() {
    let mut settings = DesktopSettings::default();
    let applied = apply_reply(&mut settings, &full_reply(), |_| true);

    assert!(applied.fallbacks.is_empty(), "{:?}", applied.fallbacks);
    assert_eq!(applied.changes.len(), Key::ALL.len());
    assert!(Key::ALL.into_iter().all(|k| settings.is_known(k)));
    // The factor passes through unrounded: the app rounds its text size once.
    assert_eq!(settings.text_scale, 1.1818);
    assert_eq!(settings.double_click, Duration::from_millis(1200));
    assert_eq!(settings.drag_threshold, 12);
    assert!(!settings.cursor_blink);
    assert_eq!(settings.cursor_blink_time, Duration::from_millis(900));
    assert_eq!(settings.cursor_blink_timeout, Duration::from_secs(4));
    assert!(!settings.enable_animations && settings.reduced_motion);
    assert!(!settings.remember_recent_files);
}

#[test]
fn an_absent_key_stays_at_its_default_and_is_reported_unset() {
    let mut settings = DesktopSettings::default();
    let applied = apply_reply(
        &mut settings,
        &reply_with(Key::TextScale, Variant::F64(1.5)),
        |_| true,
    );

    assert_eq!(settings.text_scale, 1.5);
    assert!(!settings.is_known(Key::DoubleClick));
    assert_eq!(
        settings.double_click,
        DesktopSettings::default().double_click
    );
    assert!(
        applied
            .fallbacks
            .contains(&Fallback::Unset(Key::DoubleClick))
    );
    assert!(!applied.fallbacks.contains(&Fallback::Unset(Key::TextScale)));
}

#[test]
fn a_re_read_without_a_key_resets_it() {
    let mut settings = DesktopSettings::default();
    apply_reply(&mut settings, &full_reply(), |_| true);
    let mut next = full_reply();
    next.get_mut(Key::DragThreshold.namespace())
        .expect("ns")
        .remove(Key::DragThreshold.name());

    let applied = apply_reply(&mut settings, &next, |_| true);

    assert_eq!(
        settings.drag_threshold,
        DesktopSettings::default().drag_threshold
    );
    assert!(!settings.is_known(Key::DragThreshold));
    assert!(settings.is_known(Key::DoubleClick), "the rest stay known");
    let keys: Vec<Key> = applied.changes.iter().map(|(s, _)| s.key()).collect();
    assert_eq!(keys, [Key::DragThreshold]);
    assert!(
        applied
            .fallbacks
            .contains(&Fallback::Unset(Key::DragThreshold))
    );
}

#[test]
fn an_answer_leaves_the_keys_a_newer_message_set() {
    let mut settings = DesktopSettings::default();
    settings.set(Setting::DragThreshold(99));

    let applied = apply_reply(&mut settings, &full_reply(), |k| k != Key::DragThreshold);

    assert_eq!(settings.drag_threshold, 99);
    assert_eq!(settings.text_scale, 1.1818);
    assert!(
        applied
            .changes
            .iter()
            .all(|(s, _)| s.key() != Key::DragThreshold)
    );
}

#[test]
fn a_value_of_the_wrong_type_is_refused() {
    let wrong: [(Key, Variant<'static>); 5] = [
        (Key::TextScale, Variant::I32(2)),
        (Key::TextScale, Variant::U32(2)),
        (Key::TextScale, Variant::from("1.5")),
        (Key::EnableAnimations, Variant::U32(0)),
        (Key::DoubleClick, Variant::Bool(true)),
    ];
    for (key, value) in wrong {
        let mut settings = DesktopSettings::default();
        let applied = apply_reply(&mut settings, &reply_with(key, value.clone()), |_| true);

        assert_eq!(settings, DesktopSettings::default(), "{key:?} {value:?}");
        assert!(
            applied
                .fallbacks
                .iter()
                .any(|f| matches!(f, Fallback::Invalid { key: k, .. } if *k == key)),
            "{key:?} {value:?}: {:?}",
            applied.fallbacks
        );
    }
}

#[test]
fn unusable_factors_leave_the_scale_alone() {
    for factor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut settings = DesktopSettings::default();
        settings.set(Setting::TextScale(1.25));
        let applied = apply_change(
            &mut settings,
            Key::TextScale.namespace(),
            Key::TextScale.name(),
            &Variant::F64(factor),
        );

        assert_eq!(settings.text_scale, 1.25, "{factor}");
        assert!(applied.changes.is_empty(), "{factor}");
        assert_eq!(applied.fallbacks.len(), 1, "{factor}");
    }
}

#[test]
fn durations_and_the_threshold_must_be_above_zero() {
    for (key, value) in [
        (Key::DoubleClick, Variant::I32(0)),
        (Key::DoubleClick, Variant::I32(-400)),
        (Key::CursorBlinkTime, Variant::I32(0)),
        (Key::CursorBlinkTimeout, Variant::I32(-1)),
        (Key::DragThreshold, Variant::I32(0)),
    ] {
        let mut settings = DesktopSettings::default();
        apply_change(&mut settings, key.namespace(), key.name(), &value);
        assert_eq!(settings, DesktopSettings::default(), "{key:?} {value:?}");
    }
}

#[test]
fn a_change_applies_only_to_its_own_namespace_and_key() {
    let mut settings = DesktopSettings::default();
    let elsewhere = apply_change(
        &mut settings,
        "org.freedesktop.appearance",
        "text-scaling-factor",
        &Variant::F64(1.5),
    );
    let other_key = apply_change(
        &mut settings,
        "org.gnome.desktop.interface",
        "font-name",
        &Variant::from("Sans 11"),
    );
    let own = apply_change(
        &mut settings,
        "org.gnome.desktop.interface",
        "text-scaling-factor",
        &Variant::F64(1.5),
    );

    assert_eq!(elsewhere, Default::default());
    assert_eq!(other_key, Default::default());
    assert_eq!(own.changes.len(), 1);
    assert_eq!(settings.text_scale, 1.5);
}

#[test]
fn the_same_value_again_is_no_change_but_becoming_known_is() {
    let mut settings = DesktopSettings::default();
    // The default value, now from the desktop: the view may read `is_known`.
    assert!(settings.set(Setting::DoubleClick(Duration::from_millis(400))));
    assert!(!settings.set(Setting::DoubleClick(Duration::from_millis(400))));
    assert!(settings.set(Setting::DoubleClick(Duration::from_millis(500))));
}

#[test]
fn nested_variants_are_unwrapped() {
    let mut settings = DesktopSettings::default();
    let twice = Variant::Variant(Box::new(Variant::Variant(Box::new(Variant::Bool(false)))));
    apply_change(
        &mut settings,
        Key::EnableAnimations.namespace(),
        Key::EnableAnimations.name(),
        &twice,
    );
    assert!(!settings.enable_animations);
}

#[test]
fn animations_need_the_switch_on_and_no_reduced_motion() {
    let mut settings = DesktopSettings::default();
    assert!(settings.animations());
    // An unknown reduced-motion value is no preference (the portal spec).
    for (value, animations) in [(2, true), (1, false), (0, true)] {
        apply_change(
            &mut settings,
            Key::ReducedMotion.namespace(),
            Key::ReducedMotion.name(),
            &Variant::U32(value),
        );
        assert_eq!(settings.animations(), animations, "reduced-motion {value}");
    }
    settings.set(Setting::EnableAnimations(false));
    assert!(!settings.animations());
}

#[test]
fn each_fallback_is_reported_once_per_kind_and_key() {
    let mut reported = Reported::default();
    let invalid = |value: &str| Fallback::Invalid {
        key: Key::TextScale,
        value: value.into(),
    };

    assert!(reported.first(&invalid("0")));
    assert!(
        !reported.first(&invalid("nan")),
        "another detail, same kind and key"
    );
    assert!(reported.first(&Fallback::Invalid {
        key: Key::DoubleClick,
        value: "0".into(),
    }));
    assert!(reported.first(&Fallback::NoPortal("ServiceUnknown".into())));
    assert!(!reported.first(&Fallback::NoPortal("timed out".into())));
    assert!(reported.first(&Fallback::Unset(Key::TextScale)));
}

#[test]
fn a_re_read_reports_each_differing_key_in_order() {
    let mut settings = DesktopSettings::default();
    apply_reply(&mut settings, &full_reply(), |_| true);
    let mut next = full_reply();
    let interface = next.get_mut("org.gnome.desktop.interface").expect("ns");
    interface.insert("cursor-blink".into(), owned(Variant::Bool(true)));
    interface.insert("text-scaling-factor".into(), owned(Variant::F64(2.0)));

    let applied = apply_reply(&mut settings, &next, |_| true);
    let keys: Vec<Key> = applied.changes.iter().map(|(s, _)| s.key()).collect();

    assert_eq!(keys, [Key::TextScale, Key::CursorBlink]);
    // Each change carries the settings just after it.
    assert_eq!(applied.changes[0].1.text_scale, 2.0);
    assert!(!applied.changes[0].1.cursor_blink);
    assert!(applied.changes[1].1.cursor_blink);
}
