// SPDX-License-Identifier: MIT

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread::JoinHandle;

use rune::render::{Gpu, OFFSCREEN_FORMAT, Offscreen, Renderer};
use rune::{Cx, El, Env, Runtime};

#[derive(Clone, PartialEq)]
pub struct DragImage {
    pub(crate) rgba: Arc<[u8]>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale: f32,
    pub(crate) hotspot: (f32, f32),
}

impl DragImage {
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn hotspot_point(&self) -> (f32, f32) {
        self.hotspot
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

    /// The point of the picture under the pointer, in points from its top-left.
    pub fn hotspot(self, x: f32, y: f32) -> DragIcon {
        match self.0 {
            IconKind::Image(img) => DragIcon(IconKind::Image(img.hotspot(x, y))),
            IconKind::Snapshot(r) => {
                let wrapped: Render = Box::new(move || {
                    let f = r.lock().unwrap_or_else(PoisonError::into_inner).take()?;
                    f().map(|img| img.hotspot(x, y))
                });
                DragIcon(IconKind::Snapshot(Arc::new(Mutex::new(Some(wrapped)))))
            }
        }
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

    /// Renders and returns the drag image synchronously, for unit tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn render_for_test(&self) -> Option<DragImage> {
        self.produce()
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
    rt.frame(0.0);
    let (w, h) = (
        (size.0 * scale).round().max(1.0) as u32,
        (size.1 * scale).round().max(1.0) as u32,
    );
    let mut renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
    let target = Offscreen::new(&gpu.device, w, h);
    renderer.upload_images(&gpu.device, &gpu.queue, &mut rt.images);
    renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &mut rt.text.atlas,
        &rt.instances,
        (w, h),
    );
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    renderer.draw(&mut enc, &target.view, [0.0; 4]);
    target.copy(&mut enc);
    renderer.submit(&gpu.queue, enc);
    let mut px = target.read(&gpu.device);
    unpremultiply_srgb(&mut px);
    DragImage::new(px, w, h).map(|i| i.scale(scale))
}

pub(crate) fn unpremultiply_srgb(rgba: &mut [u8]) {
    for p in rgba.as_chunks_mut::<4>().0 {
        let a = u32::from(p[3]);
        if a == 0 {
            p[..3].fill(0);
        } else if a < 255 {
            for c in &mut p[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

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
