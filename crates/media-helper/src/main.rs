// SPDX-License-Identifier: MIT

use std::{io, path::Path, process::ExitCode};
use strata_media_protocol::ipc;

mod audio;
mod parser;
mod pcm;

fn main() -> ExitCode {
    if run().is_err() {
        // Parser errors may contain input-derived strings. Never forward them to the host UI.
        eprintln!("STRATA_MEDIA:worker-failed");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run() -> Result<(), String> {
    let arguments = std::env::args_os().skip(1).map(|value| value.into_string().map_err(|_| "Invalid helper arguments".to_owned())).collect::<Result<Vec<_>, _>>()?;
    let [mode, release, commit, arguments @ ..] = arguments.as_slice() else { return Err("Invalid helper arguments".into()); };
    if release != env!("STRATA_RELEASE_TAG") || commit != env!("STRATA_BUILD_COMMIT") {
        eprintln!("STRATA_MEDIA:version");
        return Err("Version mismatch".into());
    }
    let (job, arguments) = arguments.split_last().ok_or("Missing job identity")?;
    let job = job.parse::<u64>().ok().filter(|job| *job > 0).ok_or("Invalid job identity")?;
    rustix::thread::set_no_new_privs(true).map_err(|e| e.to_string())?;
    match mode.as_str() {
        "--decode-v1" => {
            let [operation, input, output, ..] = arguments else { return Err("Invalid parser request".into()); };
            if !(input == "/input" || input.strip_prefix("/input.").is_some_and(|ext| (1..=8).contains(&ext.len()) && ext.bytes().all(|b| b.is_ascii_alphanumeric())))
                || !matches!(output.as_str(), "/dev/stdout" | "/output/result.png") {
                return Err("Parser accepts only sandbox input/output paths".into());
            }
            if operation == "preview-media" && (!Path::new("/usr/bin/ffmpeg").is_file() || !Path::new("/usr/bin/ffprobe").is_file()) {
                eprintln!("STRATA_MEDIA:ffmpeg");
                return Err("Missing FFmpeg".into());
            }
            ipc::hello(&mut io::stdout().lock(), ipc::PARSER, job, release, commit).map_err(|e| e.to_string())?;
            parser::run(arguments)
        }
        "--pcm-v1" if arguments.is_empty() => pcm::run(job, false),
        #[cfg(debug_assertions)]
        "--pcm-test-v1" if arguments.is_empty() => pcm::run(job, true),
        _ => Err("Unknown helper operation".into()),
    }
}
