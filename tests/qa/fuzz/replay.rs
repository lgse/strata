// SPDX-License-Identifier: MIT

//! Replays a sequence on the target the runner drives: a fresh fixture and home, the
//! application started on them as a scenario run starts it, each step sent in turn and an
//! observation taken at rest after every one.
//!
//! Unlike a scenario run, a step the target refuses does not end the replay: it is the
//! step's outcome, and the next step is sent. Only a target that is gone ends it.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use serde_json::{Value, json};

use super::{
    generate::Origin,
    observe::{Observation, Outcome, Rest, Rested, Sample, at_rest, milliseconds, settings, tree},
    perturb::Perturbation,
    project::{self, Places as Names},
    sequence::{Loaded, strip},
};
use crate::{
    Application, COMMAND_TIMEOUT, CONTAINER_ONLY, CONTROLLER_PANIC, Display, INPUT_LOG,
    LAUNCH_TIMEOUT, Launch, STEP_WHEN, WAIT_MARGIN,
    drive::wait_for_fs,
    fixture::{Manifest, fixture_time},
    fs_check::FsState,
    host_run, in_container,
    process::wait_for_exit,
    scenario::Scenario,
    seed,
    steps::run_steps,
    target::Target,
    variables::{Places, substitute, substitute_text, unescape, variables},
};

const SETTINGS_FILE: &str = "config/strata/settings.toml";
/// How long a target that stopped answering is given to finish exiting, so that a window
/// that was closed is told from a target that hangs.
const CLOSING: Duration = Duration::from_secs(5);

pub(crate) struct Replay<'a> {
    pub(crate) loaded: &'a Loaded,
    pub(crate) manifest: &'a Manifest,
    pub(crate) rest: Rest,
    /// Changes one observation of this replay: `perturb.rs`.
    pub(crate) perturbation: Option<&'a Perturbation>,
}

/// What a stream is a stream of: the first line of its file. Two streams are compared only
/// when they are of one sequence, one version of its alphabet and one rest rule, and the
/// normalisation map applies when their targets differ.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Header {
    /// `gtk` or `rune`.
    pub(crate) target: String,
    pub(crate) sequence: String,
    pub(crate) alphabet: String,
    pub(crate) version: u32,
    pub(crate) rest: Rest,
    /// Whether the harness changed an observation of this replay on purpose.
    pub(crate) perturbed: bool,
}

impl Header {
    fn line(&self) -> Value {
        json!({ "stream": {
            "target": self.target,
            "sequence": self.sequence,
            "alphabet": self.alphabet,
            "version": self.version,
            "rest": {
                "samples": self.rest.samples,
                "poll_ms": milliseconds(self.rest.poll),
                "budget_ms": milliseconds(self.rest.budget),
            },
            "perturbed": self.perturbed,
        } })
    }

    fn from_line(line: &Value) -> Option<Self> {
        let stream = line.get("stream")?;
        let rest = &stream["rest"];
        Some(Self {
            target: stream["target"].as_str()?.to_owned(),
            sequence: stream["sequence"].as_str()?.to_owned(),
            alphabet: stream["alphabet"].as_str()?.to_owned(),
            version: u32::try_from(stream["version"].as_u64()?).ok()?,
            rest: Rest {
                samples: u32::try_from(rest["samples"].as_u64()?).ok()?,
                poll: Duration::from_millis(rest["poll_ms"].as_u64()?),
                budget: Duration::from_millis(rest["budget_ms"].as_u64()?),
            },
            perturbed: stream["perturbed"].as_bool()?,
        })
    }

    /// Why this stream and another cannot be compared, when they cannot.
    pub(crate) fn mismatch(&self, other: &Self) -> Option<String> {
        let parts = [
            ("sequence", self.sequence == other.sequence),
            (
                "alphabet version",
                (&self.alphabet, self.version) == (&other.alphabet, other.version),
            ),
            ("rest rule", self.rest == other.rest),
        ];
        parts
            .iter()
            .find(|(_, same)| !same)
            .map(|(part, _)| format!("the two streams are not of one {part}"))
    }
}

/// How the driver and its files name a target.
pub(crate) fn target_name(target: Target) -> &'static str {
    match target {
        Target::Gtk => "gtk",
        Target::Rune => "rune",
    }
}

impl Replay<'_> {
    /// What this replay's stream is a stream of.
    fn header(&self) -> Result<Header, String> {
        let name = &self.loaded.scenario.header.scenario;
        let origin = Origin::of(name).ok_or("a sequence is named fuzz-<area>-v<version>-<seed>")?;
        Ok(Header {
            target: target_name(Target::from_environment()?).to_owned(),
            sequence: name.clone(),
            alphabet: origin.area,
            version: origin.version,
            rest: self.rest,
            perturbed: self.perturbation.is_some(),
        })
    }
}

/// The variables a sequence's header adds to the target's environment.
fn environment(scenario: &Scenario, variables: &[(String, String)]) -> Vec<(String, String)> {
    scenario
        .header
        .env
        .iter()
        .map(|(key, value)| {
            let value = substitute_text(value, variables).replace("$$", "$");
            (key.clone(), value)
        })
        .collect()
}

/// The observations of one replay, the launch first. `gone` says why the target went away,
/// when it did; the stream then ends at the step that lost it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Stream {
    pub(crate) header: Header,
    pub(crate) observations: Vec<Observation>,
    pub(crate) gone: Option<String>,
}

pub(crate) struct Replayed {
    pub(crate) stream: Stream,
    /// Every command and reply, as a scenario run's `transcript.jsonl` has them.
    pub(crate) transcript: String,
    pub(crate) log: String,
    /// Every synthetic key, motion and button, as a scenario run's `input.log` has them.
    pub(crate) input: String,
}

impl Stream {
    /// The stream's file: the header, then one observation to a line.
    pub(crate) fn text(&self) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "{}", self.header.line());
        for observation in &self.observations {
            let _ = writeln!(text, "{}", observation.line());
        }
        if let Some(gone) = &self.gone {
            let _ = writeln!(text, "{}", json!({ "gone": gone }));
        }
        text
    }

    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let mut stream = Self::default();
        for (index, line) in text.lines().enumerate() {
            let line: Value = serde_json::from_str(line)
                .map_err(|error| format!("line {}: {error}", index + 1))?;
            if index == 0 {
                stream.header = Header::from_line(&line)
                    .ok_or("line 1: a stream starts with its header, {\"stream\":…}")?;
            } else if let Some(gone) = line["gone"].as_str() {
                stream.gone = Some(gone.to_owned());
            } else {
                stream.observations.push(
                    Observation::from_line(&line)
                        .ok_or_else(|| format!("line {}: not an observation", index + 1))?,
                );
            }
        }
        Ok(stream)
    }

    pub(crate) fn count(&self, outcome: Outcome) -> usize {
        self.observations
            .iter()
            .filter(|observation| observation.outcome() == outcome.name())
            .count()
    }

    pub(crate) fn unsettled(&self) -> usize {
        self.observations
            .iter()
            .filter(|observation| !observation.settled)
            .count()
    }

    pub(crate) fn criticals(&self) -> usize {
        self.observations.iter().map(Observation::criticals).sum()
    }
}

/// The directories of one replay, made afresh and gone with it.
struct Ground {
    scratch: tempfile::TempDir,
    fixture: tempfile::TempDir,
    home: PathBuf,
    runtime: PathBuf,
}

impl Ground {
    fn new() -> Result<Self, String> {
        let directory = |prefix: &str| {
            tempfile::Builder::new()
                .prefix(prefix)
                .tempdir_in("/tmp")
                .map_err(|error| error.to_string())
        };
        let scratch = directory("strata-qa-")?;
        Ok(Self {
            home: scratch.path().join("home"),
            runtime: scratch.path().join("runtime"),
            fixture: directory("strata-qa-fixture-")?,
            scratch,
        })
    }

    fn places(&self) -> Places<'_> {
        Places {
            fixture: self.fixture.path(),
            scratch_home: &self.home,
        }
    }

    /// The files the runner's own `fs` steps may act on, and the time a tree is pinned to.
    fn files(&self, manifest: &Manifest) -> Files {
        Files {
            state: FsState {
                roots: vec![
                    self.fixture.path().to_owned(),
                    self.home.clone(),
                    self.scratch.path().to_owned(),
                ],
                ..FsState::default()
            },
            time: fixture_time(manifest),
        }
    }
}

struct Files {
    state: FsState,
    time: SystemTime,
}

/// An error is the harness's: the fixture, the display or the launch. Whatever the target
/// does once it is up is in the stream.
pub(crate) fn replay(request: &Replay) -> Result<Replayed, String> {
    if !(in_container() || host_run()) {
        return Err(CONTAINER_ONLY.to_owned());
    }
    let Loaded { scenario, run } = request.loaded;
    let header = request.header()?;
    let ground = Ground::new()?;
    let variables = variables(run, &ground.places());
    let sent = run_steps(scenario, run, &variables)?;
    if sent.len() != scenario.steps.len() {
        return Err("a sequence's steps all run in its one mode".to_owned());
    }
    let mut files = ground.files(request.manifest);
    let (fixture, manifest) = (ground.fixture.path(), request.manifest);
    seed(
        manifest,
        scenario,
        run,
        &ground.home,
        &ground.runtime,
        fixture,
    )?;
    for step in &scenario.header.setup {
        let step = unescape(&substitute(step, &variables));
        wait_for_fs(&step, fixture, files.time, &mut files.state)?;
    }
    let display = Display::start(ground.scratch.path(), &ground.runtime)?;
    let input_log = ground.scratch.path().join(INPUT_LOG);
    let launch = Launch {
        manifest,
        home: &ground.home,
        fixture,
        display: &display,
        runtime: &ground.runtime,
        log: ground.scratch.path().join("strata.log"),
        input_log: &input_log,
        extra: environment(scenario, &variables),
        bus: None,
        portal: false,
        audit: None,
    };
    let mut session = Session::start(&launch, &ground, request.rest)?;
    // A step is sent with the run's own paths in it and recorded as the sequence writes
    // it, so the action a divergence names is the same in every replay.
    let steps: Vec<Step> = scenario
        .steps
        .iter()
        .zip(&sent)
        .map(|((_, written), (_, sent))| Step {
            written,
            sent: unescape(sent),
        })
        .collect();
    let mut stream = session.run(&steps, request.perturbation, &mut files);
    stream.header = header;
    session.application.stop();
    Ok(Replayed {
        stream,
        transcript: session.transcript,
        log: fs::read_to_string(&session.application.log).unwrap_or_default(),
        input: fs::read_to_string(&input_log).unwrap_or_default(),
    })
}

struct Step<'a> {
    written: &'a Value,
    sent: Value,
}

struct Session<'a> {
    application: Application,
    fixture: &'a Path,
    settings: PathBuf,
    names: Names,
    rest: Rest,
    transcript: String,
    /// What GLib logged since the last observation, with addresses taken out.
    criticals: Vec<String>,
}

impl<'a> Session<'a> {
    /// Starts the target and waits for it to say it is ready.
    fn start(launch: &Launch, ground: &'a Ground, rest: Rest) -> Result<Self, String> {
        let mut session = Self {
            application: launch.start()?,
            fixture: ground.fixture.path(),
            settings: ground.home.join(SETTINGS_FILE),
            names: Names::new(
                &ground.fixture.path().display().to_string(),
                &ground.scratch.path().display().to_string(),
            ),
            rest,
            transcript: String::new(),
            criticals: Vec::new(),
        };
        let hello = session.application.receive(LAUNCH_TIMEOUT)?;
        session.record(&json!({ "launch": true }), &hello);
        if hello["ok"] == json!(true) {
            Ok(session)
        } else {
            Err(format!(
                "launch: {}",
                hello["error"]
                    .as_str()
                    .unwrap_or("the target was not ready")
            ))
        }
    }

    /// Observes the launch, then takes each step and observes after it, until the steps
    /// are done or the target is gone.
    fn run(
        &mut self,
        steps: &[Step],
        perturbation: Option<&Perturbation>,
        files: &mut Files,
    ) -> Stream {
        let mut stream = Stream::default();
        let (launched, mut gone) = self.observe((0, Value::Null), Outcome::Ok);
        let mut over = launched.ends_the_replay();
        stream.observations.push(launched);
        for (index, step) in steps.iter().enumerate() {
            if over {
                break;
            }
            let place = (index + 1, step.written.clone());
            let (outcome, lost) = self.take(&step.sent, files);
            let (mut observation, unseen) = if outcome.ends_the_replay() {
                (self.unseen(place, outcome), None)
            } else {
                self.observe(place, outcome)
            };
            if let Some(perturbation) = perturbation
                && outcome != Outcome::Skipped
                && let Some(before) = stream.observations.last()
            {
                perturbation.apply(step.written, &before.value, &mut observation.value);
            }
            over = observation.ends_the_replay();
            stream.observations.push(observation);
            gone = lost.or(unseen);
        }
        stream.gone = gone;
        stream
    }

    /// What a target that no longer answers did, once it has had time to finish exiting.
    fn lost(&mut self, error: String) -> (Outcome, Option<String>) {
        let exited = wait_for_exit(&mut self.application.child, CLOSING);
        gone(exited.map(|status| status.success()), error)
    }

    fn record(&mut self, command: &Value, reply: &Value) {
        let _ = writeln!(
            self.transcript,
            "{}",
            json!({ "command": command, "reply": reply })
        );
        self.criticals.extend(
            reply["criticals"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(without_addresses),
        );
    }

    fn send(&mut self, command: &Value, timeout: Duration) -> Result<Value, String> {
        let reply = self.application.send(command, timeout)?;
        self.record(command, &reply);
        Ok(reply)
    }

    /// Sends one step, or acts on the fixture for an `fs` step. The second value says why
    /// the target is gone, when the step lost it.
    fn take(&mut self, step: &Value, files: &mut Files) -> (Outcome, Option<String>) {
        if let Some(condition) = step.get(STEP_WHEN)
            && let Some(ended) = self.guard(condition)
        {
            return ended;
        }
        let step = strip(step, STEP_WHEN);
        if step.get("fs").is_some() {
            return self.act_on_files(&step, files);
        }
        let timeout = step
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .map_or(COMMAND_TIMEOUT, |ms| {
                (Duration::from_millis(ms) + WAIT_MARGIN).max(COMMAND_TIMEOUT)
            });
        match self.send(&step, timeout) {
            Ok(reply) => answered(&reply),
            Err(error) => self.lost(error),
        }
    }

    /// Asks whether a step's `when` holds. `None` when it does and the step is to be sent;
    /// otherwise how the step ended without being sent.
    fn guard(&mut self, condition: &Value) -> Option<(Outcome, Option<String>)> {
        match self.send(&json!({ "holds": condition }), COMMAND_TIMEOUT) {
            Ok(reply) if reply["result"] == json!(true) => None,
            Ok(reply) if reply["error"].is_null() => Some((Outcome::Skipped, None)),
            Ok(reply) => Some(answered(&reply)),
            Err(error) => Some(self.lost(error)),
        }
    }

    /// An `fs` step is the runner's own: one it cannot do is refused, as a step the target
    /// cannot do is.
    fn act_on_files(&mut self, step: &Value, files: &mut Files) -> (Outcome, Option<String>) {
        match wait_for_fs(step, self.fixture, files.time, &mut files.state) {
            Ok(()) => (Outcome::Ok, None),
            Err(error) => {
                self.record(step, &json!({ "ok": false, "error": error }));
                (Outcome::Refused, None)
            }
        }
    }

    /// The observation at rest after a step. The second value says why the target is gone,
    /// when it went while it was being read.
    fn observe(&mut self, step: (usize, Value), outcome: Outcome) -> (Observation, Option<String>) {
        let rest = self.rest;
        let (mut rested, error) = at_rest(&mut || self.sample(), &rest);
        if let Some(error) = error {
            let (outcome, gone) = self.lost(error);
            return (self.unseen(step, outcome), gone);
        }
        let outcome = if rested.sample.app == left_the_fixture() {
            rested.sample.app = Value::Null;
            Outcome::Left
        } else {
            outcome
        };
        let criticals = std::mem::take(&mut self.criticals);
        (Observation::new(step, outcome, criticals, rested), None)
    }

    /// The observation of a step that ended the replay before anything could be read: the
    /// files as the target left them, and no `app`, since nothing answers. After a closed
    /// window the process has exited, so the files are at rest and are compared; after a
    /// crash they are not.
    fn unseen(&mut self, step: (usize, Value), outcome: Outcome) -> Observation {
        let rested = Rested {
            sample: Sample {
                app: Value::Null,
                fs: self.names.normalise(&tree(self.fixture)),
                settings: self.names.normalise(&settings(&self.settings)),
            },
            settled: outcome == Outcome::Closed,
            polls: 0,
            waited: Duration::ZERO,
            changes: Vec::new(),
        };
        let criticals = std::mem::take(&mut self.criticals);
        Observation::new(step, outcome, criticals, rested)
    }

    fn sample(&mut self) -> Result<Sample, String> {
        let reply = self.send(&json!({ "query": "state" }), COMMAND_TIMEOUT)?;
        if let Some(error) = reply["error"].as_str() {
            return Err(error.to_owned());
        }
        let app = project::app(&reply["app"], &self.names);
        let fs = self.names.normalise(&tree(self.fixture));
        Ok(Sample {
            // Out there the listing changes with every other run on the machine, so it
            // would never come to rest: one fixed value stands for all of it.
            app: if project::outside(&app, &fs) {
                left_the_fixture()
            } else {
                app
            },
            fs,
            settings: self.names.normalise(&settings(&self.settings)),
        })
    }
}

/// What a reply says of the step it answers.
pub(crate) fn answered(reply: &Value) -> (Outcome, Option<String>) {
    let error = reply["error"].as_str();
    match error {
        _ if reply["ok"] == json!(true) => (Outcome::Ok, None),
        // The session ends itself after a panic, so no later step would be answered.
        Some(error) if error.contains(CONTROLLER_PANIC) => {
            (Outcome::Crashed, Some(error.to_owned()))
        }
        Some(_) => (Outcome::Refused, None),
        None => (Outcome::Failed, None),
    }
}

/// What a target that no longer answers did. `exited` says whether it exited, and whether
/// with success: an application whose window was closed exits so, and that is the step's
/// outcome. One that exited otherwise, or is still there and silent, crashed.
pub(crate) fn gone(exited: Option<bool>, error: String) -> (Outcome, Option<String>) {
    if exited == Some(true) {
        (Outcome::Closed, None)
    } else {
        (Outcome::Crashed, Some(error))
    }
}

/// What a sample holds for `app` while the application shows a folder outside the fixture.
fn left_the_fixture() -> Value {
    json!("outside the fixture")
}

/// A GLib message with each `0x…` address shortened to `0x`, since two runs never share one.
fn without_addresses(message: &str) -> String {
    let mut text = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(start) = rest.find("0x") {
        text.push_str(&rest[..start + 2]);
        rest =
            rest[start + 2..].trim_start_matches(|character: char| character.is_ascii_hexdigit());
    }
    text.push_str(rest);
    text
}
