// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use gdk_pixbuf::prelude::*;

use super::{
    MediaBackend, bounded_output, bounded_output_with_timeout, bounded_surface_dimensions,
    media_backends, media_command, run_media_backends, scale_embedded_thumbnail,
};

fn arguments(backend: &MediaBackend) -> String {
    media_command(backend, Path::new("/input"))
        .get_args()
        .map(|argument| argument.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn media_backends_are_deterministic_and_ordered() {
    let devices = [
        PathBuf::from("/dev/nvidiactl"),
        PathBuf::from("/dev/dri/renderD129"),
        PathBuf::from("/dev/nvidia0"),
        PathBuf::from("/dev/dri/renderD128"),
    ];

    assert_eq!(
        media_backends(&devices),
        [
            MediaBackend::VaApi("/dev/dri/renderD128".into()),
            MediaBackend::VaApi("/dev/dri/renderD129".into()),
            MediaBackend::Vulkan(0),
            MediaBackend::Vulkan(1),
            MediaBackend::Software,
        ]
    );
    assert_eq!(
        media_backends(&["/dev/nvidia0".into(), "/dev/nvidiactl".into()]),
        [MediaBackend::Vulkan(0), MediaBackend::Software]
    );
}

#[test]
fn hardware_failures_fall_back_and_first_success_stops() {
    let backends = [
        MediaBackend::VaApi("/dev/dri/renderD128".into()),
        MediaBackend::Vulkan(0),
        MediaBackend::Software,
    ];
    let mut attempts = Vec::new();
    let result = run_media_backends(&backends, |backend| {
        attempts.push(backend.clone());
        match backend {
            MediaBackend::VaApi(_) => Err(()),
            MediaBackend::Vulkan(_) => Ok(None),
            MediaBackend::Software => Ok(Some("software")),
        }
    });

    assert_eq!(result, Ok("software"));
    assert_eq!(attempts, backends);

    attempts.clear();
    let result = run_media_backends(&backends, |backend| {
        attempts.push(backend.clone());
        Ok::<_, ()>(matches!(backend, MediaBackend::Vulkan(_)).then_some("vulkan"))
    });
    assert_eq!(result, Ok("vulkan"));
    assert_eq!(attempts, &backends[..2]);
}

#[test]
fn final_software_failure_returns_the_normalization_error() {
    assert_eq!(
        run_media_backends(&[MediaBackend::Software], |_| Ok::<Option<()>, ()>(None)),
        Err("Unable to normalize media preview".to_owned())
    );
}

#[test]
fn timed_bounded_commands_stop_and_report_failure_at_their_deadline() {
    let started = Instant::now();
    let result = bounded_output_with_timeout(
        Command::new("sleep").arg("5"),
        1_024,
        Duration::from_millis(50),
    );

    assert!(result.expect("run timed command").is_none());
    assert!(started.elapsed() < Duration::from_secs(2));
    let output = bounded_output_with_timeout(
        Command::new("sh").args(["-c", "printf ok"]),
        2,
        Duration::from_secs(1),
    )
    .expect("run successful command")
    .expect("command completed before timeout");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"ok");

    let oversized = bounded_output_with_timeout(
        Command::new("sh").args(["-c", "head -c 1025 /dev/zero"]),
        1_024,
        Duration::from_secs(1),
    );
    assert!(oversized.is_err());
}

#[test]
fn pdf_surface_dimensions_stay_inside_the_parent_pixel_limit() {
    let source_width = 1_000.0;
    let source_height = 1_280.0;
    let (width, height, scale) =
        bounded_surface_dimensions(source_width, source_height, 1_400.0, 1_800.0, 2_500_000.0);

    assert!(width <= 1_400);
    assert!(height <= 1_800);
    assert!(i64::from(width) * i64::from(height) <= 2_500_000);
    assert!(source_width * scale <= f64::from(width));
    assert!(source_height * scale <= f64::from(height));
}

#[test]
fn provider_output_is_bounded_without_buffering_stderr() {
    let exact = bounded_output(Command::new("sh").args(["-c", "printf 1234"]), 4)
        .expect("read output at the limit");
    assert_eq!(exact.stdout, b"1234");

    let oversized = bounded_output(
        Command::new("sh").args(["-c", "head -c 1025 /dev/zero"]),
        1024,
    );
    assert!(oversized.is_err());

    let noisy = bounded_output(
        Command::new("sh").args(["-c", "head -c 1048576 /dev/zero >&2; printf ok"]),
        2,
    )
    .expect("discard provider stderr");
    assert_eq!(noisy.stdout, b"ok");
    assert!(noisy.stderr.is_empty());
}

#[test]
fn media_commands_select_the_backend_and_preserve_limits() {
    for backend in [
        MediaBackend::VaApi("/dev/dri/renderD129".into()),
        MediaBackend::Vulkan(1),
    ] {
        assert!(
            media_command(&backend, Path::new("/input"))
                .get_envs()
                .any(|(name, value)| name == "MALLOC_ARENA_MAX" && value == Some("1".as_ref()))
        );
    }

    let vaapi = arguments(&MediaBackend::VaApi("/dev/dri/renderD129".into()));
    assert!(vaapi.contains("-threads 1 -filter_threads 1"));
    assert!(vaapi.contains("-hwaccel vaapi -hwaccel_device /dev/dri/renderD129"));
    assert!(vaapi.contains("-hwaccel_output_format vaapi"));
    assert!(
        vaapi.contains(
            "-vf scale_vaapi=w=1280:h=1280:force_original_aspect_ratio=decrease:force_divisible_by=2:format=nv12 -c:v h264_vaapi"
        )
    );
    assert!(vaapi.contains("-c:a aac -b:a 96k -movflags +frag_keyframe+empty_moov -f mp4"));

    let vulkan = arguments(&MediaBackend::Vulkan(1));
    assert!(vulkan.contains("-threads 1 -filter_threads 1"));
    assert!(vulkan.contains("-init_hw_device vulkan=vk:1 -filter_hw_device vk"));
    assert!(vulkan.contains("-hwaccel vulkan -hwaccel_device vk"));
    assert!(vulkan.contains(
        "-vf scale_vulkan=w='if(gte(iw,ih),min(1280,trunc(iw/2)*2),-2)':h='if(gte(iw,ih),-2,min(1280,trunc(ih/2)*2))':format=nv12 -c:v h264_vulkan"
    ));
    assert!(vulkan.contains("-usage transcode -tune ull"));
    assert!(vulkan.contains("-c:a aac -b:a 96k -movflags +frag_keyframe+empty_moov -f mp4"));

    let software = arguments(&MediaBackend::Software);
    assert!(software.contains(
        "-vf scale=w=1280:h=1280:force_original_aspect_ratio=decrease,format=yuv420p -c:v libvpx -auto-alt-ref 0"
    ));
    assert!(software.contains("-threads 2 -deadline realtime -cpu-used 8"));
    assert!(software.contains("-c:a libopus -b:a 96k -f webm"));

    for command in [vaapi, vulkan, software] {
        assert!(command.contains("-max_alloc 536870912 -max_pixels 50000000"));
        assert!(command.contains("-map 0:v:0 -map 0:a:0? -sn -dn -t 30"));
        assert!(command.contains("-fpsmax 30"));
        assert!(command.contains("-b:v 2M -maxrate 3M -bufsize 4M"));
        assert!(command.ends_with("pipe:1"));
    }
}

#[test]
fn embedded_thumbnails_scale_to_the_requested_size() {
    let source = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 80, 60)
        .expect("allocate thumbnail");
    source.fill(0x3366_99ff);
    let jpeg = source
        .save_to_bufferv("jpeg", &[])
        .expect("encode thumbnail");

    let png = scale_embedded_thumbnail(&jpeg, 32).expect("scale thumbnail");
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.write(&png).expect("load scaled png");
    loader.close().expect("finish scaled png");
    let scaled = loader.pixbuf().expect("decode scaled png");

    assert_eq!((scaled.width(), scaled.height()), (32, 24));
}
