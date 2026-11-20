// SPDX-License-Identifier: MIT

//! Fixtures for the manual delete benchmarks declared in the parent module.

use super::*;

pub(super) struct Benchmark {
    pub(super) count: usize,
    pub(super) root: PathBuf,
    pub(super) unique: u128,
}

impl Benchmark {
    pub(super) fn from_env(default_count: usize) -> Result<Self, Box<dyn Error>> {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        let count = std::env::var("STRATA_DELETE_BENCH_FILES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(default_count);
        let root = std::env::var_os("STRATA_DELETE_BENCH_ROOT")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?.join("target/delete-benchmark"));
        fs::create_dir_all(&root)?;
        Ok(Self {
            count,
            root,
            unique,
        })
    }

    pub(super) fn tree(&self, label: &str) -> Result<PathBuf, Box<dyn Error>> {
        let root = self.root.join(format!("{label}-{}", self.unique));
        create_delete_benchmark_tree(&root, self.count)?;
        Ok(root)
    }

    pub(super) fn sync(&self) -> Result<(), Box<dyn Error>> {
        let handle = rustix::fs::open(
            &self.root,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )?;
        rustix::fs::syncfs(handle)?;
        Ok(())
    }

    pub(super) fn files_per_second(&self, elapsed: Duration) -> f64 {
        self.count as f64 / elapsed.as_secs_f64().max(f64::EPSILON)
    }
}

pub(super) fn worker_counts() -> Vec<usize> {
    let available_workers = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(1);
    let mut worker_counts = vec![1, 2, 4, available_workers];
    worker_counts.sort_unstable();
    worker_counts.dedup();
    worker_counts.retain(|workers| *workers <= available_workers);
    worker_counts
}

fn parent_and_name(root: &Path) -> Result<(OwnedFd, OsString), Box<dyn Error>> {
    let parent = open_local_parent_directory(root.parent().ok_or("benchmark root has no parent")?)?;
    let name = root
        .file_name()
        .ok_or("benchmark root has no name")?
        .to_owned();
    Ok((parent, name))
}

fn pinned_root(root: &Path) -> Result<PinnedEntry, Box<dyn Error>> {
    let (parent, name) = parent_and_name(root)?;
    Ok(PinnedEntry {
        parent: Arc::new(parent),
        name,
        identity: None,
    })
}

pub(super) fn delete_sequentially(root: &Path) -> Result<Duration, Box<dyn Error>> {
    let (parent, name) = parent_and_name(root)?;
    let started = Instant::now();
    glib::MainContext::default()
        .block_on(sequential_delete_local(parent, name))
        .map_err(io::Error::other)?;
    Ok(started.elapsed())
}

/// Deletes `root` with `workers`, or with the count production would choose
/// for it, and returns that production count beside the time taken.
pub(super) fn delete_with_workers(
    root: &Path,
    workers: Option<usize>,
) -> Result<(usize, Duration), Box<dyn Error>> {
    let target_root = pinned_root(root)?;
    let detected = super::super::local_delete_worker_count(std::slice::from_ref(&target_root));
    let workers = workers.unwrap_or(detected);
    let started = Instant::now();
    super::super::parallel_delete_local_blocking_with_workers(
        vec![target_root],
        Arc::new(AtomicBool::new(false)),
        workers,
        &Default::default(),
    )
    .map_err(io::Error::other)?;
    Ok((detected, started.elapsed()))
}

pub(super) fn report(
    benchmark: &Benchmark,
    serial_elapsed: Duration,
    production_workers: usize,
    measurements: &[(usize, Duration)],
) -> Result<(), Box<dyn Error>> {
    let elapsed_with = |workers: usize| {
        measurements
            .iter()
            .find(|measurement| measurement.0 == workers)
            .map(|measurement| measurement.1)
    };
    let production_elapsed =
        elapsed_with(production_workers).ok_or("missing production worker measurement")?;
    let single_elapsed = elapsed_with(1).ok_or("missing single-worker measurement")?;

    println!(
        "files: {}; production workers: {production_workers}",
        benchmark.count
    );
    println!(
        "legacy sequential: {:.3}s ({:.0} files/s)",
        serial_elapsed.as_secs_f64(),
        benchmark.files_per_second(serial_elapsed)
    );
    for (workers, elapsed) in measurements {
        println!(
            "workers={workers}: elapsed={:.3}s ({:.0} files/s)",
            elapsed.as_secs_f64(),
            benchmark.files_per_second(*elapsed)
        );
    }
    println!(
        "production parallel speedup: {:.2}x; end-to-end speedup: {:.2}x",
        single_elapsed.as_secs_f64() / production_elapsed.as_secs_f64().max(f64::EPSILON),
        serial_elapsed.as_secs_f64() / production_elapsed.as_secs_f64().max(f64::EPSILON)
    );
    Ok(())
}

fn sequential_delete_local(
    parent: OwnedFd,
    name: OsString,
) -> Pin<Box<dyn Future<Output = Result<(), String>>>> {
    Box::pin(async move {
        let step_parent = parent.try_clone().map_err(|error| error.to_string())?;
        let step_name = name.clone();
        let step = super::super::run_local_delete_step(move || {
            super::super::open_local_delete_target(&step_parent, &step_name, None, false)
        })
        .await
        .map_err(|error| error.to_string())?;
        let super::super::LocalDeleteStep::Directory { handle, children } = step else {
            return Ok(());
        };
        for child in children {
            let checked_parent = parent.try_clone().map_err(|error| error.to_string())?;
            let checked_handle = handle.try_clone().map_err(|error| error.to_string())?;
            let checked_name = name.clone();
            super::super::run_local_delete_step(move || {
                super::super::ensure_local_delete_target_unchanged(
                    &checked_parent,
                    &checked_name,
                    &checked_handle,
                )
            })
            .await
            .map_err(|error| error.to_string())?;
            sequential_delete_local(
                handle.try_clone().map_err(|error| error.to_string())?,
                child,
            )
            .await?;
        }
        super::super::run_local_delete_step(move || {
            super::super::ensure_local_delete_target_unchanged(&parent, &name, &handle)?;
            rustix::fs::unlinkat(&parent, &name, rustix::fs::AtFlags::REMOVEDIR)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())
    })
}

fn create_delete_benchmark_tree(root: &Path, count: usize) -> io::Result<()> {
    const DIRECTORIES: usize = 256;
    fs::create_dir(root)?;
    for directory in 0..DIRECTORIES.min(count.max(1)) {
        fs::create_dir(root.join(format!("dir-{directory}")))?;
    }
    for index in 0..count {
        fs::File::create(
            root.join(format!("dir-{}", index % DIRECTORIES))
                .join(format!("item-{index}")),
        )?;
    }
    Ok(())
}
