//! A drag's picture: an image, or a snapshot of a view rendered headless.
//!
//! Rune's live-frame element snapshot is `pub(crate)` (draft U1, not filed), so a snapshot
//! here is a second `Runtime` over a state the app supplies, rendered with Rune's public
//! render API (`render::Gpu::headless`, `Renderer`, `Offscreen`), as S2 measured (Q7): pixel
//! for pixel what `--snap` draws, once sRGB blending is on. It runs on a worker thread, so a
//! cold GPU (150–250 ms in S2) never stalls the UI: the drag starts at once with an empty
//! icon, and the picture lands when it is ready. That GPU device is a second one beside the
//! window's; on a machine with two GPUs it may be another adapter.
//!
//! The worker is one thread for the process, and [`stop`] joins it at `dnd::shutdown`: a render
//! still running when the process exits dies as the GPU driver is torn down under it (a
//! SIGSEGV in a session test, before this was so).

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread::JoinHandle;

use rune::render::{Gpu, OFFSCREEN_FORMAT, Offscreen, Renderer};
use rune::{Cx, El, Env, Runtime};

/// Straight (not premultiplied) sRGB RGBA pixels, `scale` pixels per point, with the point
/// under the pointer.
#[derive(Clone, PartialEq)]
pub struct DragImage {
    pub(crate) rgba: Arc<[u8]>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale: f32,
    pub(crate) hotspot: (f32, f32),
}

impl DragImage {
    /// `rgba` is `width × height` straight RGBA pixels at scale 1, with the pointer at its
    /// centre. None if the sizes don't match.
    pub fn new(rgba: impl Into<Arc<[u8]>>, width: u32, height: u32) -> Option<DragImage> {
        let rgba = rgba.into();
        let len = usize::try_from(u64::from(width) * u64::from(height) * 4).ok()?;
        (width > 0 && height > 0 && rgba.len() == len).then(|| DragImage {
            rgba,
            width,
            height,
            scale: 1.0,
            hotspot: (width as f32 / 2.0, height as f32 / 2.0),
        })
    }

    /// Pixels per point (2 for an image drawn for a scale-2 display). The hotspot keeps its
    /// place on the picture.
    pub fn scale(mut self, scale: f32) -> Self {
        if scale > 0.0 {
            self.hotspot = (
                self.hotspot.0 * self.scale / scale,
                self.hotspot.1 * self.scale / scale,
            );
            self.scale = scale;
        }
        self
    }

    /// The point of the picture under the pointer, in points from its top-left.
    pub fn hotspot(mut self, x: f32, y: f32) -> Self {
        self.hotspot = (x, y);
        self
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl fmt::Debug for DragImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DragImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("scale", &self.scale)
            .field("hotspot", &self.hotspot)
            .finish()
    }
}

type Render = Box<dyn FnOnce() -> Option<DragImage> + Send>;

/// A drag's picture.
#[derive(Clone)]
pub struct DragIcon(pub(crate) IconKind);

#[derive(Clone)]
pub(crate) enum IconKind {
    Image(DragImage),
    /// Rendered once, on a worker; taken by the first drag that uses it.
    Snapshot(Arc<Mutex<Option<Render>>>),
}

impl DragIcon {
    pub fn image(image: DragImage) -> DragIcon {
        DragIcon(IconKind::Image(image))
    }

    /// `view` over `state`, laid out at `size` points and drawn at `scale`, with the pointer
    /// at its centre: an element snapshot, built from a copy of the element's state (no
    /// hover or press, unless the copy has them).
    pub fn snapshot<T: Send + 'static>(
        state: T,
        view: fn(&T, &mut Cx<T>) -> El,
        size: (f32, f32),
        scale: f32,
        dark: bool,
    ) -> DragIcon {
        let render: Render = Box::new(move || render(state, view, size, scale, dark));
        DragIcon(IconKind::Snapshot(Arc::new(Mutex::new(Some(render)))))
    }

    /// Produces the picture, rendering a snapshot here. Blocks: call it off the UI thread.
    pub(crate) fn produce(&self) -> Option<DragImage> {
        match &self.0 {
            IconKind::Image(i) => Some(i.clone()),
            IconKind::Snapshot(r) => {
                let f = r.lock().unwrap_or_else(PoisonError::into_inner).take()?;
                catch_unwind(AssertUnwindSafe(f)).ok().flatten()
            }
        }
    }
}

impl fmt::Debug for DragIcon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            IconKind::Image(i) => i.fmt(f),
            IconKind::Snapshot(_) => f.write_str("DragIcon::snapshot"),
        }
    }
}

type Delivery = Box<dyn FnOnce(Option<DragImage>) + Send>;

/// The worker that renders snapshots, once started.
struct Worker {
    jobs: Sender<(DragIcon, Delivery)>,
    thread: JoinHandle<()>,
}

static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

/// Produces `icon`'s picture and hands it to `done`: an image at once, a snapshot on the
/// worker thread. `done` may run on either.
pub(crate) fn produce_later(icon: DragIcon, done: Delivery) -> Result<(), String> {
    if let IconKind::Image(i) = &icon.0 {
        done(Some(i.clone()));
        return Ok(());
    }
    let mut worker = WORKER.lock().unwrap_or_else(PoisonError::into_inner);
    if worker.is_none() {
        let (jobs, rx) = mpsc::channel::<(DragIcon, Delivery)>();
        let thread = std::thread::Builder::new()
            .name("dnd-icon".into())
            .spawn(move || {
                // Ends once `stop` drops the sender, after the job under way.
                for (icon, done) in rx {
                    done(icon.produce());
                }
            })
            .map_err(|e| e.to_string())?;
        *worker = Some(Worker { jobs, thread });
    }
    let w = worker.as_ref().expect("started above");
    w.jobs
        .send((icon, done))
        .map_err(|_| "the icon worker stopped".to_owned())
}

/// Stops the worker and waits for the render under way, if any, to end. On the UI thread,
/// before the event loop ends.
pub(crate) fn stop() {
    let worker = WORKER.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(Worker { jobs, thread }) = worker {
        drop(jobs);
        let _ = thread.join();
    }
}

/// One headless GPU for the process's snapshots, made on first use.
static GPU: OnceLock<Gpu> = OnceLock::new();

fn render<T: 'static>(
    state: T,
    view: fn(&T, &mut Cx<T>) -> El,
    size: (f32, f32),
    scale: f32,
    dark: bool,
) -> Option<DragImage> {
    let gpu = GPU.get_or_init(Gpu::headless);
    let mut rt = Runtime::new(state, view, size, Env::new(dark, scale));
    // A windowed app's default: a non-sRGB target with sRGB blending (S2, Q7).
    rt.set_srgb_blend(true);
    rt.frame(0.0);
    let (w, h) = (
        (size.0 * scale).round().max(1.0) as u32,
        (size.1 * scale).round().max(1.0) as u32,
    );
    let format = OFFSCREEN_FORMAT.remove_srgb_suffix();
    let mut renderer = Renderer::new(&gpu.device, format);
    renderer.set_srgb_blend(true);
    let target = Offscreen::with_format(&gpu.device, w, h, format);
    renderer.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
    renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &mut rt.text.atlas,
        &rt.instances,
        (w, h),
    );
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    renderer.draw(&mut enc, &target.view, rt.clear_color());
    target.copy(&mut enc);
    renderer.submit(&gpu.queue, enc);
    let px = target.read(&gpu.device);
    DragImage::new(px, w, h).map(|i| i.scale(scale))
}

/// Premultiplied BGRA (`wl_shm` ARGB8888 and a 32-bit X visual, little-endian) from straight
/// RGBA.
pub(crate) fn bgra_premultiplied(rgba: &[u8]) -> Vec<u8> {
    let mut out = rgba.to_vec();
    for p in out.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        let mul = |c: u8| ((u32::from(c) * a + 127) / 255) as u8;
        let (r, g, b) = (mul(p[0]), mul(p[1]), mul(p[2]));
        p[0] = b;
        p[1] = g;
        p[2] = r;
    }
    out
}

/// `image` as a buffer for an integer buffer `scale`: premultiplied BGRA, padded with clear
/// pixels to a size the scale divides (a `wl_surface` raises `invalid_size` otherwise, and
/// on winit's display that would end winit's connection), and the hotspot in the surface's
/// logical pixels. (width, height, pixels, hotspot)
pub(crate) fn buffer_for(image: &DragImage, scale: u32) -> (u32, u32, Vec<u8>, (i32, i32)) {
    let scale = scale.max(1);
    let pad = |n: u32| n.div_ceil(scale) * scale;
    let (w, h) = (pad(image.width), pad(image.height));
    let src = bgra_premultiplied(&image.rgba);
    let mut out = vec![0u8; (w * h * 4) as usize];
    let row = (image.width * 4) as usize;
    for y in 0..image.height as usize {
        let from = y * row;
        let to = y * (w as usize) * 4;
        out[to..to + row].copy_from_slice(&src[from..from + row]);
    }
    // Hotspot points → image pixels → surface-logical pixels at this buffer scale.
    let px = |pt: f32| (pt * image.scale / scale as f32).round() as i32;
    (w, h, out, (px(image.hotspot.0), px(image.hotspot.1)))
}

#[cfg(test)]
mod tests;
