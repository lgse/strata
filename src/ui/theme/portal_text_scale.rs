// SPDX-License-Identifier: MIT

//! Desktop text scaling for a D-Bus-activated process. Strata starts GTK there
//! with portals disabled, and GTK's Wayland backend then has no other source
//! for `gtk-xft-dpi`, so the factor is read from the settings portal here,
//! asynchronously, once the activating call can no longer deadlock on it.

use std::{cell::OnceCell, collections::HashMap};

use gtk::{gio, glib};

const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS_INTERFACE: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.gnome.desktop.interface";
const KEY: &str = "text-scaling-factor";

thread_local! {
    static SUBSCRIPTION: OnceCell<gio::SignalSubscription> = const { OnceCell::new() };
}

/// Only a process Strata restarted with `no-portals` needs this; a user's own
/// `no-portals` is respected.
pub(super) fn follow_if_portals_disabled() {
    if std::env::var_os(crate::child_environment::ORIGINAL_GDK_DEBUG).is_none() {
        return;
    }
    glib::MainContext::default().spawn_local(async {
        let bus = match gio::bus_get_future(gio::BusType::Session).await {
            Ok(bus) => bus,
            Err(error) => {
                tracing::debug!(%error, "session bus unavailable for text scaling");
                return;
            }
        };
        let subscription = bus.subscribe_to_signal(
            Some(PORTAL_BUS_NAME),
            Some(SETTINGS_INTERFACE),
            Some("SettingChanged"),
            Some(PORTAL_OBJECT_PATH),
            Some(NAMESPACE),
            gio::DBusSignalFlags::NONE,
            |signal| {
                if let Some(factor) = changed_text_scaling_factor(signal.parameters) {
                    apply(factor);
                }
            },
        );
        SUBSCRIPTION.with(|cell| {
            let _ = cell.set(subscription);
        });
        let reply = bus
            .call_future(
                Some(PORTAL_BUS_NAME),
                PORTAL_OBJECT_PATH,
                SETTINGS_INTERFACE,
                "ReadAll",
                Some(&(vec![NAMESPACE],).into()),
                None,
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await;
        match reply {
            Ok(reply) => {
                if let Some(factor) = read_text_scaling_factor(&reply) {
                    apply(factor);
                }
            }
            Err(error) => tracing::debug!(%error, "settings portal text scaling unavailable"),
        }
    });
}

fn apply(factor: f64) {
    let (Some(dpi), Some(settings)) = (xft_dpi(factor), gtk::Settings::default()) else {
        return;
    };
    settings.set_gtk_xft_dpi(dpi);
}

fn xft_dpi(factor: f64) -> Option<i32> {
    (factor.is_finite() && factor > 0.0).then(|| (factor * 96.0 * 1024.0).round() as i32)
}

fn read_text_scaling_factor(reply: &glib::Variant) -> Option<f64> {
    let (namespaces,) = reply.get::<(HashMap<String, HashMap<String, glib::Variant>>,)>()?;
    namespaces.get(NAMESPACE)?.get(KEY)?.get::<f64>()
}

fn changed_text_scaling_factor(parameters: &glib::Variant) -> Option<f64> {
    let (namespace, key, value) = parameters.get::<(String, String, glib::Variant)>()?;
    (namespace == NAMESPACE && key == KEY)
        .then(|| value.get::<f64>())
        .flatten()
}

#[cfg(test)]
mod tests;
