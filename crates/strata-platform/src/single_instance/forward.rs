// SPDX-License-Identifier: MIT

use std::pin::Pin;
use std::time::Duration;

use tokio::time::Instant;
use zbus::export::futures_core::Stream;
use zbus::message::Type;
use zbus::{Connection, Message, MessageStream, ObjectPath, connection};

use super::app_id::AppId;
use super::event::write_ignoring_errors;
use super::interfaces::{CALLBACK_INTERFACE, CALLBACK_PATH};
use super::launch::Launch;
use super::primary::Options;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Forward {
    /// The primary answered with this exit status.
    Answered(i32),
    /// The primary went away before answering: the launch may claim the app id itself.
    Gone(String),
    /// Anything else, said on stderr.
    Failed(String),
}

pub(super) fn primary_gone(error_name: &str, message: &str) -> bool {
    match error_name {
        "org.freedesktop.DBus.Error.ServiceUnknown"
        | "org.freedesktop.DBus.Error.NameHasNoOwner"
        | "org.freedesktop.DBus.Error.UnknownObject"
        | "org.freedesktop.DBus.Error.Disconnected" => true,
        "org.freedesktop.DBus.Error.NoReply" => message.contains("disconnected"),
        _ => false,
    }
}

pub(super) fn forward(id: &AppId, launch: &Launch, options: &Options) -> Forward {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return Forward::Failed(format!("starting the bus runtime: {e}")),
    };
    runtime.block_on(async {
        let conn =
            match tokio::time::timeout(options.bus_timeout, connection::Builder::session().build())
                .await
            {
                Ok(Ok(conn)) => conn,
                Ok(Err(e)) => {
                    return Forward::Failed(format!("connecting to the session bus: {e}"));
                }
                Err(_) => return Forward::Failed("the session bus didn't answer".into()),
            };
        call(&conn, id, launch, options.forward_timeout).await
    })
}

async fn call(conn: &Connection, id: &AppId, launch: &Launch, wait: Duration) -> Forward {
    // Before the call goes, so nothing it brings back is missed.
    let mut stream = MessageStream::from(conn);
    let message = ObjectPath::try_from(CALLBACK_PATH).and_then(|callback| {
        Message::method_call(id.object_path(), "CommandLine")
            .destination(id.as_str())
            .interface("org.gtk.Application")
            .build(&(callback, launch.wire_args(), launch.platform_data()))
    });
    let message = match message {
        Ok(m) => m,
        Err(e) => return Forward::Failed(format!("building the call: {e}")),
    };
    let serial = message.primary_header().serial_num();
    if let Err(e) = conn.send(&message).await {
        return Forward::Gone(format!("sending the launch: {e}"));
    }
    let deadline = Instant::now() + wait;
    loop {
        let next = tokio::time::timeout_at(
            deadline,
            std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)),
        )
        .await;
        let message = match next {
            Err(_) => {
                return Forward::Failed(format!("no answer in {} s", wait.as_secs_f64().round()));
            }
            Ok(None) => return Forward::Gone("the session bus closed".into()),
            Ok(Some(Err(e))) => return Forward::Failed(format!("reading the bus: {e}")),
            Ok(Some(Ok(m))) => m,
        };
        let header = message.header();
        match message.message_type() {
            Type::MethodCall => {
                let ours = header.path().is_some_and(|p| p.as_str() == CALLBACK_PATH)
                    && header
                        .interface()
                        .is_some_and(|i| i.as_str() == CALLBACK_INTERFACE);
                if !ours {
                    let _ = conn
                        .reply_dbus_error(
                            &header,
                            zbus::fdo::Error::UnknownMethod(
                                "a forwarding launch serves only its output".into(),
                            ),
                        )
                        .await;
                    continue;
                }
                let text: String = message.body().deserialize().unwrap_or_default();
                match header.member().map(|m| m.as_str()) {
                    Some("Print") => write_ignoring_errors(&mut std::io::stdout().lock(), &text),
                    Some("PrintError") => {
                        write_ignoring_errors(&mut std::io::stderr().lock(), &text);
                    }
                    _ => {}
                }
                let _ = conn.reply(&header, &()).await;
            }
            Type::MethodReturn if header.reply_serial() == Some(serial) => {
                return match message.body().deserialize::<i32>() {
                    Ok(status) => Forward::Answered(status),
                    Err(e) => Forward::Failed(format!("the answer: {e}")),
                };
            }
            Type::Error if header.reply_serial() == Some(serial) => {
                let name = header
                    .error_name()
                    .map(|n| n.as_str().to_owned())
                    .unwrap_or_default();
                let text: String = message.body().deserialize().unwrap_or_default();
                return if primary_gone(&name, &text) {
                    Forward::Gone(format!("{name}: {text}"))
                } else {
                    Forward::Failed(format!("{name}: {text}"))
                };
            }
            _ => {}
        }
    }
}
