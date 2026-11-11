// SPDX-License-Identifier: MIT

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{MEDIA_PREVIEW, Sandbox, assert_contains_all, assert_contains_none};
use crate::sandbox::{
    MediaPreviewBackend, ParseOperation, PrivateOutput, gpu_devices, polaris_gpu_available_at,
};

/// A private `dev` directory and the matching `sys/class/drm` tree.
struct DeviceTree {
    root: PrivateOutput,
}

impl DeviceTree {
    fn new() -> Self {
        Self {
            root: PrivateOutput::create().expect("create temporary device tree"),
        }
    }

    fn dev(&self) -> PathBuf {
        self.root.path().join("dev")
    }

    fn drm(&self) -> PathBuf {
        self.root.path().join("sys/class/drm")
    }

    fn render_node(&self, name: &str, pci_ids: Option<(u16, u16)>) -> PathBuf {
        create_render_node(&self.dev(), &self.drm(), name, pci_ids)
    }
}

fn create_render_node(dev: &Path, drm: &Path, name: &str, pci_ids: Option<(u16, u16)>) -> PathBuf {
    fs::create_dir_all(dev.join("dri")).expect("create DRI directory");
    let node = dev.join("dri").join(name);
    fs::write(&node, []).expect("create render node");
    if let Some((vendor, device)) = pci_ids {
        let metadata = drm.join(name).join("device");
        fs::create_dir_all(&metadata).expect("create PCI metadata");
        fs::write(metadata.join("vendor"), format!("0x{vendor:04x}\n")).expect("write PCI vendor");
        fs::write(metadata.join("device"), format!("0x{device:04x}\n")).expect("write PCI device");
    }
    node
}

fn media_preview(backend: MediaPreviewBackend, devices: &[PathBuf]) -> Sandbox<'_> {
    Sandbox::new("/home/alice/Videos/untrusted.mkv", MEDIA_PREVIEW.clone())
        .backend(backend)
        .devices(devices)
}

#[test]
fn discovers_only_supported_gpu_devices_in_stable_order() {
    let tree = DeviceTree::new();
    let dev = tree.dev();
    fs::create_dir_all(dev.join("dri")).expect("create DRI directory");
    for path in [
        "dri/renderD129",
        "dri/card0",
        "dri/renderD128",
        "dri/renderD",
        "nvidia1",
        "nvidiactl",
        "nvidia0",
        "nvidia-uvm",
        "nvidia-modeset",
        "unrelated",
    ] {
        fs::write(dev.join(path), []).expect("create device entry");
    }

    assert_eq!(
        gpu_devices(&dev, MediaPreviewBackend::Automatic),
        [
            dev.join("dri/renderD128"),
            dev.join("dri/renderD129"),
            dev.join("nvidia0"),
            dev.join("nvidia1"),
            dev.join("nvidiactl"),
        ]
    );
}

#[test]
fn every_polaris_range_uses_the_safe_default_but_remains_available_for_opt_in() {
    let tree = DeviceTree::new();
    let blocked = [0x67c0, 0x67df, 0x67e0, 0x67ff, 0x6980, 0x699f];
    for (index, device) in blocked.into_iter().enumerate() {
        tree.render_node(&format!("renderD{}", 128 + index), Some((0x1002, device)));
    }

    let devices = gpu_devices(&tree.dev(), MediaPreviewBackend::Automatic);
    assert_eq!(devices.len(), blocked.len());
    assert!(polaris_gpu_available_at(&tree.dev(), &tree.drm()));
    let command = media_preview(MediaPreviewBackend::Automatic, &devices).command();
    for device in &devices {
        assert!(
            command
                .get_args()
                .any(|argument| argument == device.as_os_str())
        );
    }
}

#[test]
fn explicit_opt_in_exposes_mixed_and_unidentified_gpus_by_policy() {
    let tree = DeviceTree::new();
    let dev = tree.dev();
    let polaris = tree.render_node("renderD128", Some((0x1002, 0x67df)));
    let modern = tree.render_node("renderD129", Some((0x1002, 0x73bf)));
    let unidentified = tree.render_node("renderD130", None);
    fs::write(dev.join("nvidia0"), []).expect("create NVIDIA device");
    fs::write(dev.join("nvidiactl"), []).expect("create NVIDIA control device");

    let render_nodes = vec![polaris, modern, unidentified];
    let with_nvidia: Vec<_> = render_nodes
        .iter()
        .cloned()
        .chain([dev.join("nvidia0"), dev.join("nvidiactl")])
        .collect();
    for (backend, expected) in [
        (MediaPreviewBackend::VaApi, render_nodes),
        (MediaPreviewBackend::Vulkan, with_nvidia.clone()),
        (MediaPreviewBackend::Automatic, with_nvidia),
        (MediaPreviewBackend::Software, Vec::new()),
    ] {
        assert_eq!(gpu_devices(&dev, backend), expected, "{backend:?}");
    }
    assert!(polaris_gpu_available_at(&dev, &tree.drm()));
}

#[test]
fn modern_and_unidentified_gpus_keep_the_accelerated_default() {
    let tree = DeviceTree::new();
    tree.render_node("renderD128", Some((0x1002, 0x73bf)));
    tree.render_node("renderD129", None);

    assert!(!polaris_gpu_available_at(&tree.dev(), &tree.drm()));
}

#[test]
fn media_sandbox_exposes_only_supplied_gpu_devices_and_sysfs() {
    let devices = [
        "/dev/dri/renderD128".into(),
        "/dev/nvidia0".into(),
        "/dev/nvidiactl".into(),
    ];
    let joined = media_preview(MediaPreviewBackend::Automatic, &devices).joined();

    let mut expected: Vec<String> = devices
        .iter()
        .map(|device| {
            let device = device.to_string_lossy();
            format!("--dev-bind-try {device} {device}")
        })
        .collect();
    expected.extend(
        [
            "--ro-bind /sys /sys",
            "preview-media /input.mkv /dev/stdout",
        ]
        .map(str::to_owned),
    );
    assert_contains_all(&joined, &expected);
    assert_contains_none(&joined, &["--cpu=10", "--bind /tmp/private-output /output"]);
}

#[test]
fn software_media_sandbox_exposes_no_gpu_devices_or_sysfs() {
    let devices = ["/dev/dri/renderD128".into(), "/dev/nvidia0".into()];
    let joined = media_preview(MediaPreviewBackend::Software, &devices).joined();

    assert_contains_none(&joined, &["--dev-bind-try", "/sys"]);
    assert!(joined.ends_with("640x800 software"));
}

#[test]
fn non_media_sandboxes_never_expose_gpu_devices_or_sysfs() {
    let devices = ["/dev/dri/renderD128".into(), "/dev/nvidia0".into()];
    let joined = Sandbox::new(
        "/home/alice/Videos/untrusted.mkv",
        ParseOperation::ThumbnailVideo,
    )
    .value(128)
    .backend(MediaPreviewBackend::Automatic)
    .devices(&devices)
    .joined();

    assert_contains_none(&joined, &["--dev-bind-try", "/sys"]);
    assert_contains_all(&joined, &["--cpu=10"]);
}
