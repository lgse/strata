//! The portal's replies, typed: a `ReadAll` answer and a `SettingChanged` body, applied to a
//! [`DesktopSettings`]. Kept free of the connection, so it is tested without a bus.

use std::collections::HashMap;
use std::time::Duration;

use zbus::{OwnedVariant, Variant};

use super::fallback::Fallback;
use super::keys::{Key, Setting};
use super::snapshot::DesktopSettings;

/// A `ReadAll` answer: namespace, then key, then value (`a{sa{sv}}`).
pub(super) type Reply = HashMap<String, HashMap<String, OwnedVariant>>;

/// What applying a reply or a change did.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Applied {
    /// Each key that changed, with the settings just after it, in [`Key::ALL`] order.
    pub changes: Vec<(Setting, DesktopSettings)>,
    pub fallbacks: Vec<Fallback>,
}

/// Applies a whole `ReadAll` answer to each key `fresh` passes (the keys no newer message has
/// set): every key it holds with a usable value, and an `Invalid` for each bad one. A key it
/// lacks goes back to its default and is no longer known (a change, if it was known), with an
/// `Unset`.
pub(super) fn apply_reply(
    settings: &mut DesktopSettings,
    reply: &Reply,
    fresh: impl Fn(Key) -> bool,
) -> Applied {
    let mut applied = Applied::default();
    for key in Key::ALL.into_iter().filter(|k| fresh(*k)) {
        match reply.get(key.namespace()).and_then(|ns| ns.get(key.name())) {
            Some(value) => apply_value(settings, key, value, &mut applied),
            None => {
                if settings.reset(key) {
                    applied.changes.push((settings.get(key), *settings));
                }
                applied.fallbacks.push(Fallback::Unset(key));
            }
        }
    }
    applied
}

/// Applies one `SettingChanged`. A namespace or key the module doesn't read changes nothing.
pub(super) fn apply_change(
    settings: &mut DesktopSettings,
    namespace: &str,
    name: &str,
    value: &Variant<'_>,
) -> Applied {
    let mut applied = Applied::default();
    if let Some(key) = Key::lookup(namespace, name) {
        apply_value(settings, key, value, &mut applied);
    }
    applied
}

fn apply_value(
    settings: &mut DesktopSettings,
    key: Key,
    value: &Variant<'_>,
    applied: &mut Applied,
) {
    match setting(key, value) {
        Some(setting) => {
            if settings.set(setting) {
                applied.changes.push((setting, *settings));
            }
        }
        None => applied.fallbacks.push(Fallback::Invalid {
            key,
            value: plain(value).to_string(),
        }),
    }
}

/// `value` as `key`'s typed setting, or None when its type is wrong or the value unusable.
/// The scale must be a `d`, its schema's type. Integer keys take any integer type, since
/// backends other than GSettings may not keep the schema's `i`.
pub(super) fn setting(key: Key, value: &Variant<'_>) -> Option<Setting> {
    let value = plain(value);
    let millis = |v: &Variant<'_>| {
        integer(v)
            .and_then(|n| u64::try_from(n).ok())
            .map(Duration::from_millis)
    };
    let setting = match key {
        Key::TextScale => match value {
            Variant::F64(f) => Setting::TextScale(*f),
            _ => return None,
        },
        Key::DoubleClick => Setting::DoubleClick(millis(value)?),
        Key::CursorBlinkTime => Setting::CursorBlinkTime(millis(value)?),
        Key::CursorBlinkTimeout => Setting::CursorBlinkTimeout(Duration::from_secs(
            integer(value).and_then(|n| u64::try_from(n).ok())?,
        )),
        Key::DragThreshold => {
            Setting::DragThreshold(integer(value).and_then(|n| u32::try_from(n).ok())?)
        }
        Key::CursorBlink => Setting::CursorBlink(boolean(value)?),
        Key::EnableAnimations => Setting::EnableAnimations(boolean(value)?),
        Key::RememberRecentFiles => Setting::RememberRecentFiles(boolean(value)?),
        // The spec: 1 reduces motion, and an unknown value means no preference.
        Key::ReducedMotion => Setting::ReducedMotion(integer(value)? == 1),
    };
    setting.is_valid().then_some(setting)
}

/// The value inside any number of variant wrappers: `Read` wraps its answer twice, and a
/// backend may wrap a value it relays.
fn plain<'a, 'v>(value: &'a Variant<'v>) -> &'a Variant<'v> {
    match value {
        Variant::Variant(inner) => plain(inner),
        v => v,
    }
}

fn integer(value: &Variant<'_>) -> Option<i64> {
    match *value {
        Variant::U8(n) => Some(n.into()),
        Variant::I16(n) => Some(n.into()),
        Variant::U16(n) => Some(n.into()),
        Variant::I32(n) => Some(n.into()),
        Variant::U32(n) => Some(n.into()),
        Variant::I64(n) => Some(n),
        Variant::U64(n) => i64::try_from(n).ok(),
        _ => None,
    }
}

fn boolean(value: &Variant<'_>) -> Option<bool> {
    match *value {
        Variant::Bool(b) => Some(b),
        _ => None,
    }
}
