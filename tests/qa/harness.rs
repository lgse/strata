// SPDX-License-Identifier: MIT

//! libtest-compatible harness that starts the costliest runs first. The `--list` lines
//! and the `test result:` summary must stay libtest's: `scripts/quality_ci.py` parses them.

use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::HashSet,
    fmt::Write as _,
    io::Write as _,
    panic::{self, AssertUnwindSafe},
    process::ExitCode,
    sync::mpsc::{self, Sender},
    time::{Duration, Instant},
};

pub const FAILED: u8 = 101;

pub const USAGE: &str = "\
usage: qa [OPTIONS] [FILTERS...]

Runs the tests whose names contain a filter (all without one), costliest first.
Run the QA scenarios through ./scripts/qa.sh; see docs/testing.md.

  --exact                 a filter is a whole test name, or a whole scenario name
                          selecting all of its runs
  --skip FILTER           leave out tests a filter would select (repeatable)
  --skip-exact NAME       leave out the test, or every run of the scenario, so named
  --ignored               run only the ignored tests (the scenario runs)
  --include-ignored       run the ignored tests too
  --test-threads N        run N tests at once (default: RUST_TEST_THREADS, else one per CPU)
  --list                  list the selected tests in cost order, then exit
  --format pretty|terse   terse lists names only
  -q, --quiet             the same as --format terse
  -h, --help              print this and exit

Accepted and ignored: --nocapture, --show-output, --report-time, --shuffle,
--shuffle-seed N, --color WHEN, -Z FLAG.
";

pub enum Outcome {
    Passed(Option<String>),
    Failed(String),
}

pub struct Skip {
    pub reason: String,
    pub notice: String,
}

pub struct Test {
    pub name: String,
    pub group: Option<String>,
    pub ignored: Option<&'static str>,
    pub skip: Option<Skip>,
    pub cost: Option<f64>,
    pub exclusive: bool,
    pub plan: Option<String>,
    pub run: Box<dyn FnOnce() -> Outcome + Send>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Arguments {
    pub filters: Vec<String>,
    pub skip: Vec<String>,
    pub skip_exact: Vec<String>,
    pub help: bool,
    pub exact: bool,
    pub list: bool,
    pub terse: bool,
    pub ignored: bool,
    pub include_ignored: bool,
    pub threads: Option<usize>,
}

impl Arguments {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let (flag, inline) = match argument.split_once('=') {
                Some((flag, value)) if flag.starts_with("--") => {
                    (flag.to_owned(), Some(value.to_owned()))
                }
                _ => (argument.clone(), None),
            };
            let mut value = || {
                inline
                    .clone()
                    .or_else(|| arguments.next())
                    .ok_or_else(|| format!("{flag} needs a value"))
            };
            match flag.as_str() {
                "--exact" => parsed.exact = true,
                "--list" => parsed.list = true,
                "--ignored" => parsed.ignored = true,
                "--include-ignored" => parsed.include_ignored = true,
                "--quiet" | "-q" => parsed.terse = true,
                "--help" | "-h" => parsed.help = true,
                "--nocapture" | "--show-output" | "--report-time" | "--shuffle" => {}
                "--skip" => parsed.skip.push(value()?),
                "--skip-exact" => parsed.skip_exact.push(value()?),
                "--test-threads" => parsed.threads = Some(threads(&value()?)?),
                "--format" => {
                    parsed.terse = match value()?.as_str() {
                        "pretty" => false,
                        "terse" => true,
                        other => {
                            return Err(format!(
                                "--format {other} is not supported; this harness prints pretty or terse"
                            ));
                        }
                    };
                }
                "--color" | "-Z" | "--shuffle-seed" => {
                    value()?;
                }
                _ if flag.starts_with("-Z") => {}
                _ if flag.starts_with('-') => {
                    return Err(format!("unrecognized option {argument}"));
                }
                _ => parsed.filters.push(argument),
            }
        }
        Ok(parsed)
    }

    fn matches(&self, test: &Test, pattern: &str) -> bool {
        if self.exact {
            named_by(test, pattern)
        } else {
            test.name.contains(pattern)
        }
    }

    pub fn selects(&self, test: &Test) -> bool {
        (self.filters.is_empty() || self.filters.iter().any(|filter| self.matches(test, filter)))
            && !self.skip.iter().any(|skip| self.matches(test, skip))
            && !self.skip_exact.iter().any(|skip| named_by(test, skip))
            && (!self.ignored || test.ignored.is_some())
    }
}

fn named_by(test: &Test, name: &str) -> bool {
    test.name == name || test.group.as_deref() == Some(name)
}

fn thread_count(arguments: &Arguments) -> Result<usize, String> {
    if let Some(count) = arguments.threads {
        return Ok(count);
    }
    match std::env::var("RUST_TEST_THREADS") {
        Ok(text) => threads(&text).map_err(|_| {
            format!("RUST_TEST_THREADS is `{text}`, which should be a positive integer")
        }),
        Err(_) => Ok(std::thread::available_parallelism().map_or(1, usize::from)),
    }
}

fn threads(text: &str) -> Result<usize, String> {
    text.parse()
        .ok()
        .filter(|count| *count > 0)
        .ok_or_else(|| format!("the thread count must be a positive number, not {text:?}"))
}

/// Uncosted tests start first, as they may be costly; ties sort by name.
pub fn starts_before(first: &Test, second: &Test) -> Ordering {
    match (first.cost, second.cost) {
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(first), Some(second)) => second.total_cmp(&first),
        (None, None) => Ordering::Equal,
    }
    .then_with(|| first.name.cmp(&second.name))
}

pub fn next_to_start(pending: &[Test], exclusive_running: bool) -> Option<usize> {
    pending
        .iter()
        .position(|test| !(test.exclusive && exclusive_running))
}

struct Finished {
    name: String,
    exclusive: bool,
    outcome: Outcome,
    elapsed: Duration,
}

thread_local! {
    static PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn capture_panics() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let captured = PANIC.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(text) = slot.as_mut() else {
                return false;
            };
            let thread = std::thread::current();
            let _ = write!(
                text,
                "thread '{}' panicked",
                thread.name().unwrap_or("<unnamed>")
            );
            if let Some(location) = info.location() {
                let _ = write!(text, " at {location}");
            }
            let _ = writeln!(
                text,
                ":\n{}",
                info.payload_as_str().unwrap_or("Box<dyn Any>")
            );
            let backtrace = std::backtrace::Backtrace::capture();
            if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
                let _ = write!(text, "stack backtrace:\n{backtrace}");
            } else {
                let _ = writeln!(
                    text,
                    "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace"
                );
            }
            true
        });
        if !captured {
            default(info);
        }
    }));
}

fn start(test: Test, finished: &Sender<Finished>) {
    let Test {
        name,
        exclusive,
        run,
        ..
    } = test;
    let started = Instant::now();
    let sender = finished.clone();
    let thread_name = name.clone();
    let spawned = std::thread::Builder::new()
        .name(name.clone())
        .spawn(move || {
            PANIC.with(|slot| *slot.borrow_mut() = Some(String::new()));
            let outcome = panic::catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|_| {
                Outcome::Failed(
                    PANIC
                        .with(|slot| slot.borrow_mut().take())
                        .unwrap_or_default(),
                )
            });
            let _ = sender.send(Finished {
                name: thread_name,
                exclusive,
                outcome,
                elapsed: started.elapsed(),
            });
        });
    if let Err(error) = spawned {
        let _ = finished.send(Finished {
            name,
            exclusive,
            outcome: Outcome::Failed(format!("its thread did not start: {error}")),
            elapsed: started.elapsed(),
        });
    }
}

fn emit(text: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(text.as_bytes());
    let _ = stdout.flush();
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

pub fn run(mut tests: Vec<Test>, arguments: &Arguments) -> ExitCode {
    if arguments.help {
        emit(USAGE);
        return ExitCode::SUCCESS;
    }
    let mut names = HashSet::new();
    if let Some(test) = tests.iter().find(|test| !names.insert(test.name.as_str())) {
        eprintln!("error: two tests are named {}", test.name);
        return ExitCode::from(FAILED);
    }
    let total = tests.len();
    tests.retain(|test| arguments.selects(test));
    let filtered_out = total - tests.len();
    tests.sort_by(starts_before);
    if arguments.list {
        let mut listing = String::new();
        for test in &tests {
            let _ = writeln!(listing, "{}: test", test.name);
        }
        if !arguments.terse {
            let _ = writeln!(
                listing,
                "\n{} test{}, 0 benchmarks",
                tests.len(),
                plural(tests.len())
            );
        }
        emit(&listing);
        return ExitCode::SUCCESS;
    }

    let threads = match thread_count(arguments) {
        Ok(threads) => threads,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(FAILED);
        }
    };
    let started = Instant::now();
    let include_ignored = arguments.ignored || arguments.include_ignored;
    let mut header = format!("\nrunning {} test{}\n", tests.len(), plural(tests.len()));
    let mut held = String::new();
    let mut notices = HashSet::new();
    let mut pending = Vec::new();
    let mut ignored = 0;
    for test in tests {
        let reason = match (&test.ignored, &test.skip) {
            (Some(reason), _) if !include_ignored => (*reason).to_owned(),
            (_, Some(skip)) => {
                if notices.insert(skip.notice.clone()) {
                    let _ = writeln!(header, "{}", skip.notice);
                }
                skip.reason.clone()
            }
            _ => {
                if let Some(plan) = &test.plan {
                    let _ = writeln!(header, "{plan}");
                }
                pending.push(test);
                continue;
            }
        };
        ignored += 1;
        let _ = writeln!(held, "test {} ... ignored, {reason}", test.name);
    }
    emit(&(header + &held));

    capture_panics();
    let (sender, receiver) = mpsc::channel();
    let mut running = 0;
    let mut exclusive_running = false;
    let mut passed = 0;
    let mut failures = Vec::new();
    loop {
        while running < threads
            && let Some(index) = next_to_start(&pending, exclusive_running)
        {
            let test = pending.remove(index);
            exclusive_running |= test.exclusive;
            running += 1;
            start(test, &sender);
        }
        if running == 0 {
            break;
        }
        let finished = receiver.recv().expect("a running test always reports back");
        running -= 1;
        exclusive_running &= !finished.exclusive;
        let seconds = finished.elapsed.as_secs_f64();
        match finished.outcome {
            Outcome::Passed(line) => {
                passed += 1;
                let mut text = format!("test {} ... ok <{seconds:.3}s>\n", finished.name);
                if let Some(line) = line {
                    let _ = writeln!(text, "{line}");
                }
                emit(&text);
            }
            Outcome::Failed(details) => {
                emit(&format!(
                    "test {} ... FAILED <{seconds:.3}s>\n",
                    finished.name
                ));
                failures.push((finished.name, details));
            }
        }
    }

    let mut summary = String::new();
    if !failures.is_empty() {
        summary.push_str("\nfailures:\n\n");
        for (name, details) in &failures {
            let _ = writeln!(summary, "---- {name} stdout ----\n{}\n", details.trim_end());
        }
        summary.push_str("\nfailures:\n");
        for (name, _) in &failures {
            let _ = writeln!(summary, "    {name}");
        }
    }
    let _ = writeln!(
        summary,
        "\ntest result: {}. {passed} passed; {} failed; {ignored} ignored; 0 measured; \
         {filtered_out} filtered out; finished in {:.2}s\n",
        if failures.is_empty() { "ok" } else { "FAILED" },
        failures.len(),
        started.elapsed().as_secs_f64(),
    );
    emit(&summary);
    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(FAILED)
    }
}
