// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use super::{
    MediaBackend, media_backends, media_command, run_command_with_timeout, run_media_backends,
};

fn arguments(backend: &MediaBackend) -> String {
    media_command(
        backend,
        Path::new("/input"),
        Path::new("/output/result.media"),
    )
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
            MediaBackend::Vulkan(_) => Ok(false),
            MediaBackend::Software => Ok(true),
        }
    });

    assert_eq!(result, Ok(()));
    assert_eq!(attempts, backends);

    attempts.clear();
    let result = run_media_backends(&backends, |backend| {
        attempts.push(backend.clone());
        Ok::<_, ()>(matches!(backend, MediaBackend::Vulkan(_)))
    });
    assert_eq!(result, Ok(()));
    assert_eq!(attempts, &backends[..2]);
}

#[test]
fn final_software_failure_returns_the_normalization_error() {
    assert_eq!(
        run_media_backends(&[MediaBackend::Software], |_| Ok::<_, ()>(false)),
        Err("Unable to normalize media preview".to_owned())
    );
}

#[test]
fn timed_commands_stop_and_report_failure_at_their_deadline() {
    let started = Instant::now();
    let result =
        run_command_with_timeout(Command::new("sleep").arg("5"), Duration::from_millis(50));

    assert!(!result.expect("run timed command"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        run_command_with_timeout(&mut Command::new("true"), Duration::from_secs(1))
            .expect("run successful command")
    );
}

#[test]
fn media_commands_select_the_backend_and_preserve_limits() {
    for backend in [
        MediaBackend::VaApi("/dev/dri/renderD129".into()),
        MediaBackend::Vulkan(1),
    ] {
        assert!(
            media_command(&backend, Path::new("/input"), Path::new("/output"))
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
    assert!(vaapi.contains("-c:a aac -b:a 96k -f mp4"));

    let vulkan = arguments(&MediaBackend::Vulkan(1));
    assert!(vulkan.contains("-threads 1 -filter_threads 1"));
    assert!(vulkan.contains("-init_hw_device vulkan=vk:1 -filter_hw_device vk"));
    assert!(vulkan.contains("-hwaccel vulkan -hwaccel_device vk"));
    assert!(vulkan.contains(
        "-vf scale_vulkan=w='if(gte(iw,ih),min(1280,trunc(iw/2)*2),-2)':h='if(gte(iw,ih),-2,min(1280,trunc(ih/2)*2))':format=nv12 -c:v h264_vulkan"
    ));
    assert!(vulkan.contains("-usage transcode -tune ull"));
    assert!(vulkan.contains("-c:a aac -b:a 96k -f mp4"));

    let software = arguments(&MediaBackend::Software);
    assert!(
        software
            .contains("-vf scale=w=1280:h=1280:force_original_aspect_ratio=decrease -c:v libvpx")
    );
    assert!(software.contains("-threads 2 -deadline realtime -cpu-used 8"));
    assert!(software.contains("-c:a libopus -b:a 96k -f webm"));

    for command in [vaapi, vulkan, software] {
        assert!(command.contains("-map 0:v:0 -map 0:a:0? -sn -dn -t 30"));
        assert!(command.contains("-fpsmax 30"));
        assert!(command.contains("-b:v 2M -maxrate 3M -bufsize 4M"));
        assert!(command.ends_with("-y /output/result.media"));
    }
}
