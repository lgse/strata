// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

use std::sync::Arc;

use ashpd::{
    MaybeAppID, PortalError, WindowIdentifierType,
    desktop::{
        Response,
        file_chooser::{OpenFileOptions, SaveFileOptions, SaveFilesOptions, SelectedFiles},
    },
    zvariant::{Optional, OwnedObjectPath},
};

use super::{ChooserRequest, FileChooserBackend, TrackedRequest};

pub(super) struct FileChooserInterface {
    backend: FileChooserBackend,
}

impl FileChooserInterface {
    pub(super) fn new() -> Self {
        Self {
            backend: FileChooserBackend::default(),
        }
    }

    async fn begin(
        &self,
        connection: &zbus::Connection,
        handle: &OwnedObjectPath,
    ) -> ashpd::backend::Result<TrackedRequest> {
        let token = handle.as_str().to_owned();
        if !token.starts_with("/org/freedesktop/portal/desktop/request/") {
            return Err(PortalError::InvalidArgument(
                "invalid file chooser request path".into(),
            ));
        }
        let tracked = self.backend.requests.begin(token.clone())?;
        connection
            .object_server()
            .at(
                handle.clone(),
                RequestInterface {
                    token,
                    requests: self.backend.requests.clone(),
                },
            )
            .await?;
        Ok(tracked)
    }

    async fn finish(
        &self,
        connection: &zbus::Connection,
        handle: &OwnedObjectPath,
        tracked: TrackedRequest,
        request: ashpd::backend::Result<ChooserRequest>,
    ) -> ashpd::backend::Result<Response<SelectedFiles>> {
        let result = match request {
            Ok(request) => self.backend.choose(&tracked, request).await,
            Err(error) => Err(error),
        };
        // Only the method completing the request removes its interface. Close merely
        // cancels it, avoiding ashpd 0.13.13's concurrent double-removal race.
        connection
            .object_server()
            .remove::<RequestInterface, _>(handle)
            .await?;
        if tracked.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(Response::cancelled());
        }
        match result {
            Ok(files) => Ok(Response::ok(files)),
            Err(PortalError::Cancelled(_)) => Ok(Response::cancelled()),
            Err(error) => Err(error),
        }
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooserInterface {
    #[zbus(property(emits_changed_signal = "const"), name = "version")]
    fn version(&self) -> u32 {
        super::FILE_CHOOSER_VERSION
    }

    #[zbus(out_args("response", "results"))]
    async fn open_file(
        &self,
        handle: OwnedObjectPath,
        _app_id: Optional<MaybeAppID>,
        parent: Optional<WindowIdentifierType>,
        title: String,
        options: OpenFileOptions,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> ashpd::backend::Result<Response<SelectedFiles>> {
        let tracked = self.begin(connection, &handle).await?;
        let request = super::open_request(handle.to_string(), parent.into(), &title, options).await;
        self.finish(connection, &handle, tracked, request).await
    }

    #[zbus(out_args("response", "results"))]
    async fn save_file(
        &self,
        handle: OwnedObjectPath,
        _app_id: Optional<MaybeAppID>,
        parent: Optional<WindowIdentifierType>,
        title: String,
        options: SaveFileOptions,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> ashpd::backend::Result<Response<SelectedFiles>> {
        let tracked = self.begin(connection, &handle).await?;
        let request =
            super::save_file_request(handle.to_string(), parent.into(), &title, options).await;
        self.finish(connection, &handle, tracked, request).await
    }

    #[zbus(out_args("response", "results"))]
    async fn save_files(
        &self,
        handle: OwnedObjectPath,
        _app_id: Optional<MaybeAppID>,
        parent: Optional<WindowIdentifierType>,
        title: String,
        options: SaveFilesOptions,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> ashpd::backend::Result<Response<SelectedFiles>> {
        let tracked = self.begin(connection, &handle).await?;
        let request =
            super::save_files_request(handle.to_string(), parent.into(), &title, options).await;
        self.finish(connection, &handle, tracked, request).await
    }
}

struct RequestInterface {
    token: String,
    requests: Arc<super::RequestTracker>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl RequestInterface {
    async fn close(&self) {
        if self.requests.cancel(&self.token) {
            let token = self.token.clone();
            glib::MainContext::default().invoke(move || crate::ui::cancel_chooser(&token));
        }
    }
}
