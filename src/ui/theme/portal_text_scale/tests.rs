// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use gtk::glib::{self, prelude::*};

use super::{changed_text_scaling_factor, read_text_scaling_factor, xft_dpi};

#[test]
fn read_all_reply_yields_the_desktop_text_scaling_factor() {
    let interface = HashMap::from([
        ("font-name".to_owned(), "Adwaita Sans 11".to_variant()),
        ("text-scaling-factor".to_owned(), 1.1818_f64.to_variant()),
    ]);
    let reply = (HashMap::from([(
        "org.gnome.desktop.interface".to_owned(),
        interface,
    )]),);
    let reply = reply.to_variant();
    assert_eq!(reply.type_().as_str(), "(a{sa{sv}})");
    let factor = read_text_scaling_factor(&reply);

    assert_eq!(factor, Some(1.1818));
    assert_eq!(factor.and_then(xft_dpi), Some(116_176));
}

#[test]
fn read_all_reply_without_the_factor_changes_nothing() {
    let reply = (HashMap::<String, HashMap<String, glib::Variant>>::new(),);

    assert_eq!(read_text_scaling_factor(&reply.to_variant()), None);
}

#[test]
fn setting_changed_follows_only_the_text_scaling_factor() {
    let changed = |namespace: &str, key: &str, value: glib::Variant| {
        changed_text_scaling_factor(&(namespace, key, value).to_variant())
    };

    assert_eq!(
        changed(
            "org.gnome.desktop.interface",
            "text-scaling-factor",
            1.5_f64.to_variant()
        ),
        Some(1.5)
    );
    assert_eq!(
        changed(
            "org.gnome.desktop.interface",
            "font-name",
            "Sans 11".to_variant()
        ),
        None
    );
    assert_eq!(
        changed(
            "org.freedesktop.appearance",
            "text-scaling-factor",
            1.5_f64.to_variant()
        ),
        None
    );
}

#[test]
fn unusable_factors_leave_the_dpi_alone() {
    for factor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(xft_dpi(factor), None, "{factor}");
    }
}
