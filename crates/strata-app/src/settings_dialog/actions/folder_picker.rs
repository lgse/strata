// SPDX-License-Identifier: MIT

//! Asynchronous folder picker via the XDG Desktop Portal (`org.freedesktop.portal.FileChooser`),
//! replacing GTK's `gtk::FileDialog::select_folder` for custom-action import and export.

use std::{
    cell::RefCell,
    collections::HashMap,
    future::Future,
    path::PathBuf,
    pin::Pin,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

use gio::prelude::*;
use glib::variant::{ObjectPath, ToVariant};
use strata_core::i18n::tr;
use strata_fs::services::UNEXPECTED_SYSTEM_ERROR;

const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER_IFACE: &str = "org.freedesktop.portal.FileChooser";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";
const REQUEST_PREFIX: &str = "/org/freedesktop/portal/desktop/request";
const CALL_TIMEOUT_MSEC: i32 = 15_000;

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Asks the desktop portal to pick a single folder without blocking the UI thread.
///
/// Returns `Ok(Some(path))` on acceptance, `Ok(None)` when cancelled, or `Err(message)` if the
/// portal call fails.
pub(super) async fn select_folder(title: String) -> Result<Option<PathBuf>, String> {
    let connection = gio::bus_get_future(gio::BusType::Session)
        .await
        .map_err(dbus_error)?;
    let token = format!(
        "strata_actions_{}_{}",
        std::process::id(),
        NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
    );
    let expected = expected_request_path(connection.unique_name().as_deref(), &token);

    let slot = Rc::new(RefCell::new(ResponseSlot::default()));
    let mut subscriptions = Vec::new();
    if let Some(path) = expected.as_deref() {
        subscriptions.push(subscribe_response(&connection, path, Rc::clone(&slot)));
    }

    let reply = call_open_folder(&connection, &title, &token).await;
    let request_path = match reply {
        Ok(variant) => variant
            .get::<(ObjectPath,)>()
            .map(|(path,)| path.as_str().to_owned())
            .ok_or_else(|| tr(UNEXPECTED_SYSTEM_ERROR))?,
        Err(error) => {
            drop(subscriptions);
            return Err(dbus_error(error));
        }
    };

    if expected.as_deref() != Some(request_path.as_str()) {
        subscriptions.push(subscribe_response(
            &connection,
            &request_path,
            Rc::clone(&slot),
        ));
    }

    let response = ResponseFuture { slot }.await;
    drop(subscriptions);
    Ok(response)
}

async fn call_open_folder(
    connection: &gio::DBusConnection,
    title: &str,
    token: &str,
) -> Result<glib::Variant, glib::Error> {
    let mut options: HashMap<&str, glib::Variant> = HashMap::new();
    options.insert("handle_token", token.to_variant());
    options.insert("directory", true.to_variant());
    options.insert("modal", true.to_variant());
    let params = ("", title, options).to_variant();
    connection
        .call_future(
            Some(PORTAL_BUS),
            PORTAL_PATH,
            FILE_CHOOSER_IFACE,
            "OpenFile",
            Some(&params),
            None,
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MSEC,
        )
        .await
}

fn expected_request_path(unique_name: Option<&str>, token: &str) -> Option<String> {
    let sender = unique_name?.trim_start_matches(':').replace('.', "_");
    (!sender.is_empty()).then(|| format!("{REQUEST_PREFIX}/{sender}/{token}"))
}

fn subscribe_response(
    connection: &gio::DBusConnection,
    path: &str,
    slot: Rc<RefCell<ResponseSlot>>,
) -> gio::SignalSubscription {
    connection.subscribe_to_signal(
        Some(PORTAL_BUS),
        Some(REQUEST_IFACE),
        Some("Response"),
        Some(path),
        None,
        gio::DBusSignalFlags::NONE,
        move |signal| {
            let parsed = parse_portal_response(signal.parameters);
            let mut guard = slot.borrow_mut();
            if guard.result.is_none() {
                guard.result = Some(parsed);
                if let Some(waker) = guard.waker.take() {
                    waker.wake();
                }
            }
        },
    )
}

pub(super) fn parse_portal_response(parameters: &glib::Variant) -> Option<PathBuf> {
    let (code, results) = parameters.get::<(u32, HashMap<String, glib::Variant>)>()?;
    if code != 0 {
        return None;
    }
    let uris = results.get("uris")?.get::<Vec<String>>()?;
    let uri = uris.first()?;
    gio::File::for_uri(uri).path()
}

fn dbus_error(error: glib::Error) -> String {
    strata_fs::services::gio_error_message(&error)
}

#[derive(Default)]
struct ResponseSlot {
    result: Option<Option<PathBuf>>,
    waker: Option<Waker>,
}

struct ResponseFuture {
    slot: Rc<RefCell<ResponseSlot>>,
}

impl Future for ResponseFuture {
    type Output = Option<PathBuf>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut guard = self.slot.borrow_mut();
        if let Some(result) = guard.result.take() {
            Poll::Ready(result)
        } else {
            guard.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
