// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::atomic::Ordering,
};

use ashpd::desktop::ResponseType;

use super::*;

struct PrivateBus(Child);

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "requires dbus-daemon; uses a private session bus, no display"]
fn close_is_idempotent_and_completion_removes_each_request_once()
-> Result<(), Box<dyn std::error::Error>> {
    let _serial = crate::test_support::ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("main context test lock");
    let context = glib::MainContext::default();
    let _owner = context.acquire()?;
    let mut bus = PrivateBus(
        Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()?,
    );
    let mut address = String::new();
    BufReader::new(bus.0.stdout.take().expect("private bus stdout")).read_line(&mut address)?;
    async_io::block_on(async {
        let connection = zbus::connection::Builder::address(address.trim())?
            .build()
            .await?;
        let client = zbus::connection::Builder::address(address.trim())?
            .build()
            .await?;
        let service = FileChooserInterface::new();
        let first =
            OwnedObjectPath::try_from("/org/freedesktop/portal/desktop/request/client_a/shared")?;
        let second =
            OwnedObjectPath::try_from("/org/freedesktop/portal/desktop/request/client_b/shared")?;
        let tracked = service.begin(&connection, &first).await?;
        assert!(service.begin(&connection, &first).await.is_err());
        let other = service.begin(&connection, &second).await?;
        let request = zbus::Proxy::new(
            &client,
            connection
                .unique_name()
                .expect("registered connection name")
                .to_owned(),
            first.clone(),
            "org.freedesktop.impl.portal.Request",
        )
        .await?;
        request.call::<_, _, ()>("Close", &()).await?;
        request.call::<_, _, ()>("Close", &()).await?;
        assert!(tracked.cancelled.load(Ordering::SeqCst));
        assert!(!other.cancelled.load(Ordering::SeqCst));
        let response = service
            .finish(
                &connection,
                &first,
                tracked,
                Err(PortalError::Cancelled("test".into())),
            )
            .await?;
        assert_eq!(response.response_type(), ResponseType::Cancelled);
        assert!(request.call::<_, _, ()>("Close", &()).await.is_err());
        let replacement = service.begin(&connection, &first).await?;
        service
            .finish(
                &connection,
                &first,
                replacement,
                Err(PortalError::Cancelled("test".into())),
            )
            .await?;
        service
            .finish(
                &connection,
                &second,
                other,
                Err(PortalError::Cancelled("test".into())),
            )
            .await?;
        assert!(
            service
                .backend
                .requests
                .active
                .lock()
                .expect("request tracker lock")
                .is_empty()
        );
        Ok::<(), Box<dyn std::error::Error>>(())
    })?;
    while context.pending() {
        context.iteration(false);
    }
    Ok(())
}
