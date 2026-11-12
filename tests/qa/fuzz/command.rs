// SPDX-License-Identifier: MIT

//! The fuzz commands: which sequences a command means, replaying them several at once, and
//! the shrink of one.

use std::{
    collections::VecDeque, panic::AssertUnwindSafe, path::Path, process::ExitCode, sync::Mutex,
};

use super::{
    alphabet,
    compare::{Divergence, compare},
    corpus,
    generate::{GENERATOR, generate, name, seed_of},
    options::{Command, Options, USAGE},
    replay::{Replay, Replayed, Stream, replay},
    report::{self, Evidence, Examined, Run},
    rules::{self, CAUSES, EXCEPTIONS, Ignored},
    sequence::{Loaded, PREFIX, Sequence},
    shrink::{CONFIRMATIONS, shrink},
};
use crate::{
    REPOSITORY,
    fixture::{Manifest, manifest},
    harness,
    run::run_scenario,
    target::Target,
};

const USAGE_ERROR: u8 = 2;

/// One sequence to examine, and the area whose corpus and artifacts it belongs to.
struct Job {
    area: String,
    sequence: Sequence,
}

pub(crate) fn main(arguments: impl IntoIterator<Item = String>) -> ExitCode {
    let options = match Options::parse(arguments) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            return ExitCode::from(USAGE_ERROR);
        }
    };
    let done = match options.command {
        Command::Check => check(),
        Command::Generate => generated(&options).map(|sequence| print!("{}", sequence.text())),
        Command::Scenario => return scenario(&options.files[0]),
        Command::Compare => compare_files(&options.files[0], &options.files[1]),
        Command::Run | Command::Replay => examine_all(&options),
        Command::Shrink => shrink_one(&options),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(harness::FAILED)
        }
    }
}

/// Everything the fuzzer reads from the tree, loaded as a run would load it: a line for
/// each alphabet and each corpus, and the counts of the map and the mask.
pub(crate) fn checked() -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for area in alphabet::areas() {
        let alphabet = alphabet::load(&area)?;
        lines.push(format!(
            "FUZZ alphabet {area} v{}: {} actions; {}",
            alphabet.version,
            alphabet.actions.len(),
            alphabet.about
        ));
    }
    for area in corpus::areas() {
        let sequences = corpus::sequences(&area)?;
        for sequence in &sequences {
            sequence.load()?;
        }
        lines.push(format!(
            "FUZZ corpus {area}: {} sequences, {} seeds parked",
            sequences.len(),
            corpus::parked(&area)?.len()
        ));
    }
    lines.push(rule_counts(&Ignored::load(false)?));
    Ok(lines)
}

fn check() -> Result<(), String> {
    for line in checked()? {
        println!("{line}");
    }
    Ok(())
}

pub(crate) fn rule_counts(ignored: &Ignored) -> String {
    format!(
        "FUZZ rules: {} normalisation rules ({}/{}); {} calibration masks ({}/{})",
        rules::counts(&ignored.rules, &EXCEPTIONS),
        super::DATA,
        rules::MAP,
        rules::counts(&ignored.masks, &CAUSES),
        super::DATA,
        rules::MASK,
    )
}

fn generated(options: &Options) -> Result<Sequence, String> {
    let area = options.alphabet.as_deref().unwrap_or_default();
    Ok(generate(
        &alphabet::load(area)?,
        options.seeds[0],
        options.length,
    ))
}

/// The ordinary runner on a sequence file: every step sent as a scenario's is, and the
/// first one refused ends the run.
fn scenario(path: &Path) -> ExitCode {
    let loaded = match Sequence::read(path).and_then(|sequence| sequence.load()) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(harness::FAILED);
        }
    };
    match run_scenario(path, &loaded.run.label) {
        harness::Outcome::Passed(line) => {
            println!("{}", line.unwrap_or_default());
            ExitCode::SUCCESS
        }
        harness::Outcome::Failed(details) => {
            println!("{details}");
            ExitCode::from(harness::FAILED)
        }
    }
}

/// Two kept streams of one sequence, compared as a run compares its replays.
fn compare_files(left: &Path, right: &Path) -> Result<(), String> {
    let read = |path: &Path| {
        std::fs::read_to_string(path)
            .map_err(|error| error.to_string())
            .and_then(|text| Stream::parse(&text))
            .map_err(|error| format!("{}: {error}", path.display()))
    };
    let ignored = Ignored::load(true)?;
    println!("{}", rule_counts(&ignored));
    match compare(&read(left)?, &read(right)?, &ignored) {
        None => {
            println!("AGREE {} and {}", left.display(), right.display());
            Ok(())
        }
        Some(divergence) => Err(report::differ_line("the streams", &divergence)),
    }
}

/// The area a sequence name holds: `fuzz-<area>-<seed>`.
fn area_of(name: &str) -> Option<String> {
    seed_of(name)?;
    let area = name.strip_prefix(&format!("{PREFIX}-"))?;
    Some(area[..area.len().checked_sub(17)?].to_owned())
}

/// The sequences a command means. A parked seed is named and left out.
fn jobs(options: &Options, run: &mut Run) -> Result<Vec<Job>, String> {
    if options.command == Command::Run {
        return generated_jobs(options, run);
    }
    run.what = "kept sequences".to_owned();
    if !options.files.is_empty() {
        return options.files.iter().map(|path| file_job(path)).collect();
    }
    let areas = match &options.alphabet {
        Some(area) => vec![area.clone()],
        None => corpus::areas(),
    };
    let mut jobs = Vec::new();
    for area in areas {
        let parked = corpus::parked(&area)?;
        for sequence in corpus::sequences(&area)? {
            let seed = seed_of(&sequence.name);
            match parked.iter().find(|parked| Some(parked.seed) == seed) {
                Some(parked) => run.parked(&sequence.name, parked),
                None => jobs.push(Job {
                    area: area.clone(),
                    sequence,
                }),
            }
        }
    }
    Ok(jobs)
}

fn generated_jobs(options: &Options, run: &mut Run) -> Result<Vec<Job>, String> {
    let area = options.alphabet.clone().unwrap_or_default();
    let alphabet = alphabet::load(&area)?;
    run.what = format!(
        "alphabet {area} v{}, generator {GENERATOR}, {} actions",
        alphabet.version, options.length
    );
    let parked = corpus::parked(&area)?;
    let mut jobs = Vec::new();
    for seed in &options.seeds {
        match parked.iter().find(|parked| parked.seed == *seed) {
            Some(parked) => run.parked(&name(&area, *seed), parked),
            None => jobs.push(Job {
                area: area.clone(),
                sequence: generate(&alphabet, *seed, options.length),
            }),
        }
    }
    Ok(jobs)
}

fn file_job(path: &Path) -> Result<Job, String> {
    let sequence = Sequence::read(path)?;
    let area = area_of(&sequence.name)
        .ok_or_else(|| format!("{}: a sequence is named fuzz-<area>-<seed>", path.display()))?;
    Ok(Job { area, sequence })
}

/// What every replay of a command shares.
struct Bench<'a> {
    options: &'a Options,
    manifest: Manifest,
    ignored: Ignored,
    target: Target,
}

impl<'a> Bench<'a> {
    fn new(options: &'a Options) -> Result<Self, String> {
        Ok(Self {
            options,
            manifest: manifest(),
            // One target replayed against itself: the map's rules do not apply.
            ignored: Ignored::load(true)?,
            target: Target::from_environment()?,
        })
    }

    fn replay(&self, loaded: &Loaded, perturbed: bool) -> Result<Replayed, String> {
        replay(&Replay {
            loaded,
            manifest: &self.manifest,
            rest: self.options.rest,
            perturbation: self.options.perturbation.as_ref().filter(|_| perturbed),
        })
    }
}

fn examine_all(options: &Options) -> Result<(), String> {
    let bench = Bench::new(options)?;
    let mut run = Run::new(options, bench.target, &bench.ignored);
    let jobs = jobs(options, &mut run)?;
    run.start(jobs.len());
    let queue = Mutex::new(jobs.into_iter().collect::<VecDeque<Job>>());
    let run = Mutex::new(run);
    std::thread::scope(|scope| {
        for _ in 0..options.jobs {
            scope.spawn(|| {
                loop {
                    let Some(job) = lock(&queue).pop_front() else {
                        break;
                    };
                    // The runner's own setup panics on a fault of the harness; one sequence's
                    // is that sequence's error, and the others go on.
                    let examined =
                        std::panic::catch_unwind(AssertUnwindSafe(|| examine(&job, &bench)))
                            .unwrap_or_else(|_| {
                                let mut failed = Examined::new(&job.area, &job.sequence);
                                failed.error =
                                    Some("the replay panicked; the reason is above".to_owned());
                                failed
                            });
                    lock(&run).add(&examined);
                }
            });
        }
    });
    run.into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .finish()
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Replays one sequence `--repeats` times, one after another, and compares each replay with
/// the first: once with nothing left out, for calibration, and once as the map and the mask
/// have it. Only the last replay is perturbed, when the command perturbs.
fn examine(job: &Job, bench: &Bench) -> Examined {
    let mut examined = Examined::new(&job.area, &job.sequence);
    let loaded = match job.sequence.load() {
        Ok(loaded) => loaded,
        Err(error) => {
            examined.error = Some(error);
            return examined;
        }
    };
    let repeats = bench.options.repeats;
    let mut replays: Vec<Replayed> = Vec::new();
    for index in 0..repeats {
        match bench.replay(&loaded, index + 1 == repeats) {
            Ok(replayed) => replays.push(replayed),
            Err(error) => {
                examined.error = Some(error);
                break;
            }
        }
    }
    if let Some((first, others)) = replays.split_first() {
        let against = |ignored: &Ignored| {
            others
                .iter()
                .find_map(|other| compare(&first.stream, &other.stream, ignored))
        };
        examined.unmasked = against(&Ignored::default());
        examined.divergence = against(&bench.ignored);
    }
    let evidence = Evidence {
        sequence: &job.sequence,
        replays: &replays,
        target: bench.target,
    };
    examined.kept = report::keep(&examined, &evidence, bench.options);
    examined.streams = replays
        .into_iter()
        .map(|replayed| replayed.stream)
        .collect();
    examined
}

/// Replays the steps twice at once, the second replay perturbed when the command says so,
/// and compares the two.
fn diverges(
    sequence: &Sequence,
    bench: &Bench,
) -> Result<(Option<Divergence>, [Replayed; 2]), String> {
    let loaded = sequence.load()?;
    let (left, right) = std::thread::scope(|scope| {
        let left = scope.spawn(|| bench.replay(&loaded, false));
        let right = bench.replay(&loaded, true);
        (left.join(), right)
    });
    let left = left.map_err(|_| "a replay panicked".to_owned())??;
    let right = right?;
    let divergence = compare(&left.stream, &right.stream, &bench.ignored);
    Ok((divergence, [left, right]))
}

fn shrink_one(options: &Options) -> Result<(), String> {
    let bench = Bench::new(options)?;
    let sequence = match options.files.first() {
        Some(path) => Sequence::read(path)?,
        None => generated(options)?,
    };
    let area = area_of(&sequence.name)
        .ok_or_else(|| format!("{}: a sequence is named fuzz-<area>-<seed>", sequence.name))?;
    let mut run = Run::new(options, bench.target, &bench.ignored);
    run.what = format!("shrinking {}", sequence.name);
    run.start(1);
    let (found, replays) = diverges(&sequence, &bench)?;
    let Some(found) = found else {
        return Err(format!(
            "{}: two replays agree, so there is nothing to shrink",
            sequence.name
        ));
    };
    println!("{}", report::differ_line(&sequence.name, &found));
    let mut last = replays;
    let shrunk = shrink(&sequence.steps, &found, options.limit, &mut |steps| {
        let (divergence, replays) = diverges(&sequence.with_steps(steps.to_vec()), &bench)?;
        if divergence.is_some() {
            last = replays;
        }
        Ok(divergence)
    })?;
    let shortest = sequence.with_steps(shrunk.steps.clone());
    let evidence = Evidence {
        sequence: &sequence,
        replays: &last,
        target: bench.target,
    };
    report::shrunk(&area, &shrunk, &shortest, &evidence);
    if !shrunk.is_confirmed() {
        return Err(format!(
            "{}: the shortest sequence diverged {} times out of {CONFIRMATIONS}: this is \
             noise for calibration, not a divergence to file",
            sequence.name, shrunk.confirmed
        ));
    }
    if options.corpus {
        let path = corpus::directory(&area).join(format!("{}.jsonl", shortest.name));
        std::fs::create_dir_all(corpus::directory(&area))
            .and_then(|()| std::fs::write(&path, shortest.text()))
            .map_err(|error| format!("{}: {error}", path.display()))?;
        println!(
            "FUZZ corpus: wrote {}",
            path.strip_prefix(REPOSITORY).unwrap_or(&path).display()
        );
    }
    Ok(())
}
