// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::{
    Application, BINARY, COMMAND_TIMEOUT, DEFAULT_WAIT, FFMPEG_TIMEOUT, Launch, POLL_INTERVAL,
    QA_ENVIRONMENT, REPOSITORY, RUNNER_OWNED_ENVIRONMENT, XDG_DIRECTORIES,
    drive::Context,
    fs_check::{PngCheck, TarText, contained},
    fs_helpers::{check_png, percent_decoded},
    peers,
    process::{spawn, terminate},
    variables::PYTHON,
};

/// `services::remote_download::DOWNLOAD_PREFIX`; only such directories may be discarded.
const DOWNLOAD_PREFIX: &str = "strata-download-";
const LAUNCH_TIMEOUT_ARGUMENTS: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaunchStep {
    pub(crate) launch: Vec<String>,
    #[serde(default)]
    pub(crate) percent_path: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Relaunch {
    pub(crate) directory: Option<String>,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    stdout: Option<String>,
    stderr: Option<String>,
}

impl Relaunch {
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.stdout.is_some() != self.stderr.is_some() {
            return Err("a restart gives both stdout and stderr, or neither".to_owned());
        }
        for stream in self.stdout.iter().chain(&self.stderr) {
            if !matches!(stream.as_str(), "closed" | "null") {
                return Err(format!("a stream is closed or null, not {stream:?}"));
            }
        }
        if let Some(name) = self.env.keys().find(|name| {
            RUNNER_OWNED_ENVIRONMENT.contains(&name.as_str())
                || XDG_DIRECTORIES.iter().any(|(variable, _)| variable == name)
                || QA_ENVIRONMENT.iter().any(|(variable, _)| variable == name)
        }) {
            return Err(format!(
                "a restart may not set {name}, which the runner owns"
            ));
        }
        Ok(())
    }

    pub(crate) fn streams(&self) -> Option<String> {
        Some(format!(
            "stdout={},stderr={}",
            self.stdout.as_deref()?,
            self.stderr.as_deref()?
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortalOpenStep {
    pub(crate) portal_open: String,
    #[serde(default)]
    options: peers::OpenOptions,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortalResponseStep {
    pub(crate) portal_response: PortalCheck,
    pub(crate) timeout_ms: Option<u64>,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortalCheck {
    pub(crate) done: bool,
    pub(crate) status: Option<u32>,
    pub(crate) names: Option<Vec<String>>,
    pub(crate) paths: Option<Vec<String>>,
    pub(crate) keep: Option<String>,
    pub(crate) png: Option<PngCheck>,
    #[serde(default)]
    pub(crate) discard: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HttpStep {
    http: String,
    #[serde(rename = "as")]
    pub(crate) name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HttpRequestsStep {
    http_requests: usize,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForeignStep {
    pub(crate) foreign: ForeignPaths,
    #[serde(rename = "as")]
    pub(crate) name: String,
    timeout_ms: Option<u64>,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForeignPaths {
    pub(crate) paths: Vec<String>,
    #[serde(default)]
    move_only: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ForeignLabelStep {
    foreign_label: String,
    timeout_ms: Option<u64>,
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ZipCryptoStep {
    zipcrypto: ZipCryptoArchive,
    #[serde(rename = "as")]
    pub(crate) name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ZipCryptoArchive {
    path: String,
    password: String,
    member: String,
    contents: TarText,
    #[serde(default)]
    deflated: bool,
}

#[derive(Default)]
pub(crate) struct Peers {
    portal: Option<peers::PortalRequest>,
    server: Option<peers::ImageServer>,
    foreign: Option<peers::ForeignSource>,
}

pub(crate) enum PeerError {
    Failed(String),
    Harness(String),
}

fn wait_until(
    timeout: Option<u64>,
    message: Option<&str>,
    mut ready: impl FnMut() -> bool,
) -> Result<(), PeerError> {
    let deadline = Instant::now() + timeout.map_or(DEFAULT_WAIT, Duration::from_millis);
    while !ready() {
        if Instant::now() >= deadline {
            return Err(PeerError::Failed(format!(
                "timed out{}",
                message
                    .map(|message| format!(" waiting for {message}"))
                    .unwrap_or_default()
            )));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    Ok(())
}

pub(crate) fn peer_step(
    application: &mut Application,
    peers: &mut Peers,
    context: &Context,
    object: &Map<String, Value>,
    notes: &mut Vec<(String, Value)>,
) -> Option<Result<Option<(Value, Value)>, PeerError>> {
    fn parsed<T: serde::de::DeserializeOwned>(
        object: &Map<String, Value>,
        kind: &str,
    ) -> Result<T, PeerError> {
        serde_json::from_value(Value::Object(object.clone()))
            .map_err(|error| PeerError::Harness(format!("invalid {kind} step: {error}")))
    }
    let keep = |notes: &mut Vec<(String, Value)>, name: &str, value: Value| {
        notes.retain(|(note, _)| note != name);
        notes.push((name.to_owned(), value));
    };
    Some(if object.contains_key("launch") {
        parsed::<LaunchStep>(object, "launch").and_then(|step| {
            launch_instance(context.launch, &step)
                .map(|()| None)
                .map_err(PeerError::Failed)
        })
    } else if object.contains_key("portal_open") {
        parsed::<PortalOpenStep>(object, "portal_open").and_then(|step| {
            let bus = context
                .launch
                .bus
                .ok_or_else(|| PeerError::Harness("a portal run needs a session bus".to_owned()))?;
            let request = peers::PortalRequest::send(bus, &step.portal_open, &step.options)
                .map_err(PeerError::Failed)?;
            peers.portal = Some(request);
            Ok(None)
        })
    } else if object.contains_key("portal_response") {
        parsed::<PortalResponseStep>(object, "portal_response").and_then(|step| {
            let request = peers
                .portal
                .as_mut()
                .ok_or_else(|| PeerError::Harness("no portal request was sent".to_owned()))?;
            portal_response(request, &step, notes).map(|()| None)
        })
    } else if object.contains_key("http") {
        parsed::<HttpStep>(object, "http").and_then(|step| {
            let server = peers::ImageServer::start(gtk::glib::base64_decode(&step.http))
                .map_err(PeerError::Harness)?;
            keep(notes, &step.name, json!(server.url));
            peers.server = Some(server);
            Ok(None)
        })
    } else if object.contains_key("http_requests") {
        parsed::<HttpRequestsStep>(object, "http_requests").and_then(|step| {
            let made = peers
                .server
                .as_ref()
                .ok_or_else(|| PeerError::Harness("no image server runs".to_owned()))?
                .requests();
            if made == step.http_requests {
                Ok(None)
            } else {
                Err(PeerError::Failed(format!(
                    "{}: {made} requests, not {}",
                    step.message.as_deref().unwrap_or("request count"),
                    step.http_requests
                )))
            }
        })
    } else if object.contains_key("foreign") {
        parsed::<ForeignStep>(object, "foreign").and_then(|step| {
            if peers.foreign.is_some() {
                return Err(PeerError::Harness(
                    "a foreign drag source already runs; stop it first".to_owned(),
                ));
            }
            let mut arguments = Vec::new();
            if step.foreign.move_only {
                arguments.push("--move-only".to_owned());
            }
            arguments.extend(step.foreign.paths.iter().cloned());
            let source = peers::ForeignSource::start(
                PYTHON,
                Path::new(REPOSITORY),
                &arguments,
                &context.launch.helper_environment(),
                &context.launch.log.with_file_name("foreign-drag-source.log"),
            )
            .map_err(PeerError::Harness)?;
            let found = wait_until(step.timeout_ms, step.message.as_deref(), || {
                source.bounds().is_some()
            });
            let request = json!({ "keep": source.bounds(), "as": step.name });
            peers.foreign = Some(source);
            found?;
            application
                .send(&request, COMMAND_TIMEOUT)
                .map(|reply| Some((request, reply)))
                .map_err(PeerError::Harness)
        })
    } else if object.contains_key("foreign_label") {
        parsed::<ForeignLabelStep>(object, "foreign_label").and_then(|step| {
            let source = peers
                .foreign
                .as_ref()
                .ok_or_else(|| PeerError::Harness("no foreign drag source runs".to_owned()))?;
            wait_until(step.timeout_ms, step.message.as_deref(), || {
                source.label().as_deref() == Some(step.foreign_label.as_str())
            })
            .map(|()| None)
        })
    } else if object.contains_key("foreign_stop") {
        drop(peers.foreign.take());
        Ok(None)
    } else if object.contains_key("zipcrypto") {
        parsed::<ZipCryptoStep>(object, "zipcrypto").and_then(|step| {
            let archive = &step.zipcrypto;
            let path = context.fixture.join(&archive.path);
            contained(&path, &[context.fixture.to_path_buf()]).map_err(PeerError::Harness)?;
            peers::write_zipcrypto(
                &path,
                &archive.password,
                &archive.member,
                &archive.contents.bytes(),
                archive.deflated,
            )
            .and_then(|()| peers::zipcrypto_collision(&path))
            .map(|collision| {
                keep(notes, &step.name, json!(collision));
                None
            })
            .map_err(PeerError::Harness)
        })
    } else {
        let message = object.get("fail")?;
        Err(PeerError::Failed(
            message.as_str().unwrap_or_default().to_owned(),
        ))
    })
}

fn launch_instance(launch: &Launch, step: &LaunchStep) -> Result<(), String> {
    use std::os::unix::ffi::OsStringExt as _;
    let arguments = step
        .launch
        .iter()
        .map(|argument| {
            if step.percent_path {
                percent_decoded(argument).map(std::ffi::OsString::from_vec)
            } else {
                Ok(argument.into())
            }
        })
        .collect::<Result<Vec<std::ffi::OsString>, String>>()?;
    let output = File::options()
        .create(true)
        .append(true)
        .open(launch.log.with_file_name("launches.log"))
        .map_err(|error| error.to_string())?;
    let errors = output.try_clone().map_err(|error| error.to_string())?;
    // A second instance hands its arguments over and exits; its trace is all startup.
    let (mut command, _capture) = launch.command(BINARY)?;
    let mut child = spawn(
        command
            .args(&arguments)
            .current_dir(launch.fixture)
            .env_clear()
            .envs(launch.helper_environment())
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(errors),
    )
    .map_err(|error| format!("cannot start {BINARY}: {error}"))?;
    let deadline = Instant::now() + LAUNCH_TIMEOUT_ARGUMENTS;
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => return Err(format!("strata {arguments:?} exited with {status}")),
            None if Instant::now() >= deadline => {
                terminate(&mut child);
                return Err(format!(
                    "strata {arguments:?} ran past {} s",
                    LAUNCH_TIMEOUT_ARGUMENTS.as_secs()
                ));
            }
            None => std::thread::sleep(POLL_INTERVAL),
        }
    }
}

fn portal_response(
    request: &mut peers::PortalRequest,
    step: &PortalResponseStep,
    notes: &mut Vec<(String, Value)>,
) -> Result<(), PeerError> {
    let check = &step.portal_response;
    let failed = |detail: String| {
        PeerError::Failed(match &step.message {
            Some(message) => format!("{message}: {detail}"),
            None => detail,
        })
    };
    if !check.done {
        return match request.poll() {
            None => Ok(()),
            Some(answer) => Err(failed(format!("the portal already answered {answer:?}"))),
        };
    }
    let deadline = Instant::now() + step.timeout_ms.map_or(DEFAULT_WAIT, Duration::from_millis);
    let answer = loop {
        if let Some(answer) = request.poll() {
            break answer.clone().map_err(failed)?;
        }
        if Instant::now() >= deadline {
            return Err(failed("timed out waiting for the answer".to_owned()));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    if let Some(status) = check.status
        && answer.status != status
    {
        return Err(failed(format!(
            "the status is {}, not {status}",
            answer.status
        )));
    }
    if let Some(names) = &check.names {
        let mut chosen: Vec<String> = answer
            .paths
            .iter()
            .filter_map(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        let mut wanted = names.clone();
        chosen.sort();
        chosen.dedup();
        wanted.sort();
        wanted.dedup();
        if chosen != wanted {
            return Err(failed(format!("the chosen names are {chosen:?}")));
        }
    }
    if let Some(paths) = &check.paths {
        let chosen: Vec<String> = answer
            .paths
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        if chosen != *paths {
            return Err(failed(format!("the chosen paths are {chosen:?}")));
        }
    }
    if check.keep.is_none() && check.png.is_none() && !check.discard {
        return Ok(());
    }
    let first = answer
        .paths
        .first()
        .ok_or_else(|| failed("the answer chose no file".to_owned()))?;
    if let Some(name) = &check.keep {
        notes.retain(|(note, _)| note != name);
        notes.push((name.clone(), json!(first.display().to_string())));
    }
    let checked = check
        .png
        .as_ref()
        .map_or(Ok(()), |png| check_png(first, png).map_err(failed));
    if check.discard {
        // Only a `services::remote_download` directory straight in the
        // temporary directory, named with its `DOWNLOAD_PREFIX`.
        let parent = first
            .parent()
            .filter(|parent| {
                parent.parent() == Some(std::env::temp_dir().as_path())
                    && parent
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with(DOWNLOAD_PREFIX))
            })
            .ok_or_else(|| {
                PeerError::Harness(format!("{} is no Strata download", first.display()))
            })?;
        fs::remove_dir_all(parent)
            .map_err(|error| PeerError::Harness(format!("{}: {error}", parent.display())))?;
    }
    checked
}

pub(crate) fn dbus_send(launch: &Launch, arguments: &[String]) -> Result<(), String> {
    if launch.bus.is_none() {
        return Err("dbus_send needs the header's session_bus".to_owned());
    }
    let mut child = spawn(
        Command::new("dbus-send")
            .arg("--session")
            .args(arguments)
            .env_clear()
            .envs(launch.helper_environment())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )
    .map_err(|error| format!("cannot run dbus-send: {error}"))?;
    let deadline = Instant::now() + FFMPEG_TIMEOUT;
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                let mut errors = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut stderr, &mut errors);
                }
                return Err(format!("dbus-send exited with {status}: {}", errors.trim()));
            }
            None if Instant::now() >= deadline => {
                terminate(&mut child);
                return Err(format!("dbus-send ran past {} s", FFMPEG_TIMEOUT.as_secs()));
            }
            None => std::thread::sleep(POLL_INTERVAL),
        }
    }
}
