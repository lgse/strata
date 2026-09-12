// SPDX-License-Identifier: MIT

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MediaPreviewSize {
    pub width: i32,
    pub height: i32,
}

impl MediaPreviewSize {
    pub const MAX_EDGE: i32 = 1280;

    pub fn new(width: i32, height: i32) -> Self {
        Self { width: width.clamp(16, Self::MAX_EDGE), height: height.clamp(16, Self::MAX_EDGE) }
    }

    pub fn for_viewport(width: i32, height: i32, scale: i32) -> Self {
        Self::new(width.saturating_mul(scale), height.saturating_mul(scale))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MediaPreviewBackend { Automatic, VaApi, Vulkan, Software }

impl MediaPreviewBackend {
    pub fn argument(self) -> &'static str {
        match self { Self::Automatic => "automatic", Self::VaApi => "vaapi", Self::Vulkan => "vulkan", Self::Software => "software" }
    }

    pub fn from_argument(value: &str) -> Option<Self> {
        match value { "automatic" => Some(Self::Automatic), "vaapi" => Some(Self::VaApi), "vulkan" => Some(Self::Vulkan), "software" => Some(Self::Software), _ => None }
    }
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) { self.0.store(true, Ordering::Release); }
    pub fn is_cancelled(&self) -> bool { self.0.load(Ordering::Acquire) }
}
