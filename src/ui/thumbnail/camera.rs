// SPDX-License-Identifier: MIT

use std::io::Write;

use super::*;

const MAX_ICON_BYTES: usize = 1024 * 1024;
const ICON_TIMEOUT: Duration = Duration::from_secs(15);

thread_local! {
    static RETRY_PENDING: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn retry_after_cancel() {
    if !ACTIVE_REQUESTS.with(|requests| {
        requests
            .borrow()
            .values()
            .any(|active| active.deferred.is_some() && active.image.upgrade().is_some())
    }) {
        return;
    }
    if !RETRY_PENDING.with(|pending| pending.replace(true)) {
        // Unbind can cancel during GTK layout; viewport ranking belongs in idle.
        glib::idle_add_local_once(|| {
            RETRY_PENDING.with(|pending| pending.set(false));
            retry_deferred_thumbnails();
        });
    }
}

#[cfg(test)]
use super::viewport::prioritize_queue;

pub(super) async fn render(path: &Path, cancellation: &Cancellation) -> Result<Vec<u8>, String> {
    if cancellation.is_cancelled() {
        return Err("Camera thumbnail cancelled".into());
    }
    let uri = path.to_str().ok_or("Invalid camera URI")?;
    let file = gio::File::for_uri(uri);
    let download = async {
        let info = file
            .query_info_future(
                "preview::icon",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
            )
            .await
            .map_err(|error| error.to_string())?;
        let icon = info
            .attribute_object("preview::icon")
            .and_then(|object| object.downcast::<gio::LoadableIcon>().ok())
            .ok_or("The camera did not provide a thumbnail")?;
        let stream = load_icon_stream(&icon)
            .await
            .map_err(|error| error.to_string())?;
        let bytes = read_icon(&stream, MAX_ICON_BYTES).await?;
        stream
            .close_future(glib::Priority::DEFAULT)
            .await
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(bytes)
    };
    let cancelled = async {
        loop {
            if cancellation.is_cancelled() {
                return Err("Camera thumbnail cancelled".to_owned());
            }
            glib::timeout_future(Duration::from_millis(20)).await;
        }
    };
    let bytes = glib::future_with_timeout(
        ICON_TIMEOUT,
        futures_lite::future::race(download, cancelled),
    )
    .await
    .map_err(|_| "Camera thumbnail timed out".to_owned())??;
    let cancellation = cancellation.clone();
    // Camera preview icons are compressed, untrusted inputs too. Only the
    // sandbox's normalized PNG is handed to GTK, never the original icon bytes.
    background::render(move || {
        if cancellation.is_cancelled() {
            return Err("Camera thumbnail cancelled".into());
        }
        let mut input = tempfile::Builder::new()
            .prefix("strata-camera-thumbnail-")
            .tempfile()
            .map_err(|error| error.to_string())?;
        input.write_all(&bytes).map_err(|error| error.to_string())?;
        render_thumbnail(input.path(), ThumbnailKind::Image, &cancellation)
            .map(|thumbnail| thumbnail.png)
    })
    .await
    .map_err(|_| "Camera thumbnail worker failed".to_owned())?
}

// gio 0.22 treats load_finish's optional content type as a non-null GString.
// GVfs preview icons may return a stream without a type. Request only the stream
// until the binding is corrected; decoding still happens exclusively in the sandbox.
#[expect(
    unsafe_code,
    reason = "GIO's nullable content-type output is incorrectly bound as non-null in gio 0.22"
)]
async fn load_icon_stream(icon: &gio::LoadableIcon) -> Result<gio::InputStream, glib::Error> {
    use glib::translate::*;
    type Completion = gio::GioFutureResult<Result<gio::InputStream, glib::Error>>;

    unsafe extern "C" fn completed(
        source: *mut glib::gobject_ffi::GObject,
        result: *mut gio::ffi::GAsyncResult,
        data: glib::ffi::gpointer,
    ) {
        // SAFETY: load_async receives this box exactly once; its callback retains
        // ownership even when GioFuture is dropped, and runs on the initiating context.
        let completion = unsafe { Box::from_raw(data.cast::<Completion>()) };
        let mut error = std::ptr::null_mut();
        // SAFETY: GIO supplies the matching live source/result; the type output is optional.
        let raw_stream = unsafe {
            gio::ffi::g_loadable_icon_load_finish(
                source.cast(),
                result,
                std::ptr::null_mut(),
                &mut error,
            )
        };
        // SAFETY: load_finish transfers ownership of the stream, or returns null.
        let stream: Option<gio::InputStream> = unsafe { from_glib_full(raw_stream) };
        let result = if !error.is_null() {
            // SAFETY: the non-null GError is transferred to this caller by load_finish.
            Err(unsafe { from_glib_full(error) })
        } else {
            stream.ok_or_else(|| {
                glib::Error::new(
                    gio::IOErrorEnum::Failed,
                    "Camera returned no thumbnail stream",
                )
            })
        };
        completion.resolve(result);
    }

    gio::GioFuture::new(icon, |icon, cancellable, completion: Completion| {
        // SAFETY: source/cancellable are live; GIO retains them until its callback.
        // Completion is boxed for that single callback, including after cancellation.
        unsafe {
            gio::ffi::g_loadable_icon_load_async(
                icon.to_glib_none().0,
                256,
                cancellable.to_glib_none().0,
                Some(completed),
                Box::into_raw(Box::new(completion)).cast(),
            );
        }
    })
    .await
}

async fn read_icon(stream: &impl IsA<gio::InputStream>, limit: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    loop {
        let remaining = limit - output.len();
        let bytes = stream
            .read_bytes_future(8192.min(remaining + 1), glib::Priority::DEFAULT)
            .await
            .map_err(|error| error.to_string())?;
        if bytes.is_empty() {
            return Ok(output);
        }
        if bytes.len() > remaining {
            return Err("Camera thumbnail exceeds the size limit".into());
        }
        output.extend_from_slice(&bytes);
    }
}

#[cfg(test)]
mod tests;
