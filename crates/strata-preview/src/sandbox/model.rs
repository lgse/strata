// SPDX-License-Identifier: MIT

use std::path::Path;

use crate::services::{ModelEncoding, ModelFormat, ModelMesh, ModelRender};

pub(super) fn read_mesh(render: ModelRender, path: &Path) -> Result<Option<ModelMesh>, String> {
    let invalid = || "The preview renderer produced invalid image data".to_owned();
    let bytes = super::read_private_output(path, 256).map_err(|_| invalid())?;
    let mesh: Option<ModelMesh> = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let Some(mesh) = mesh else {
        if render.format == ModelFormat::Stl {
            return Err(invalid());
        }
        return Ok(None);
    };
    let format_matches = matches!(
        (render.format, mesh.encoding),
        (
            ModelFormat::Stl,
            ModelEncoding::BinaryStl | ModelEncoding::AsciiStl
        ) | (ModelFormat::ThreeMf, ModelEncoding::ThreeMf)
    );
    if !format_matches
        || mesh.view != render.view
        || mesh.triangles == 0
        || mesh.triangles > crate::services::model_preview::MAX_MODEL_TRIANGLES
    {
        return Err(invalid());
    }
    Ok(Some(mesh))
}

pub(super) fn wait_for_thumbnail(
    renderer: super::Renderer,
    cancellation: &super::Cancellation,
    timeout: std::time::Duration,
    progress: &mut dyn FnMut(),
    output: std::sync::Arc<super::PrivateOutput>,
    hold: std::sync::Arc<crate::services::file_activity::Hold>,
) -> Result<std::process::ExitStatus, String> {
    wait_with_reaper(
        renderer,
        cancellation,
        timeout,
        progress,
        (output, hold),
        || {},
    )
}

fn wait_with_reaper<G: Send + 'static>(
    mut renderer: super::Renderer,
    cancellation: &super::Cancellation,
    timeout: std::time::Duration,
    progress: &mut dyn FnMut(),
    guards: G,
    before_reap: impl FnOnce() + Send + 'static,
) -> Result<std::process::ExitStatus, String> {
    let result = super::monitor_renderer(&mut renderer, cancellation, timeout, progress);
    if result.is_err() {
        super::kill_renderer(&mut renderer);
        // SIGKILL may remain pending in uninterruptible I/O. The caller's worker and
        // admission slot must not wait for reap, while its input/output guards stay alive.
        std::thread::spawn(move || {
            before_reap();
            let _waited = renderer.child.wait();
            renderer.await_namespace();
            drop(guards);
        });
    }
    result
}

#[cfg(test)]
mod tests;
