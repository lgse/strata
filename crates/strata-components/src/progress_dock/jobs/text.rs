//! The Jobs dashboard's words and text helpers (Strata's `src/ui/jobs.rs`).

use std::fmt;
use std::rc::Rc;

use super::super::format;
use super::{JobRow, JobStatus};

/// A job's status words from its status and elapsed time.
pub type StatusPhrase = Rc<dyn Fn(JobStatus, &str) -> String>;

/// The dashboard's words. Defaults are Strata's English.
#[derive(Clone)]
pub struct JobsWords {
    /// "2 jobs running" (the footer).
    pub jobs_running: Rc<dyn Fn(usize) -> String>,
    /// "1 queued".
    pub queued: Rc<dyn Fn(usize) -> String>,
    /// "3 jobs finished".
    pub jobs_finished: Rc<dyn Fn(usize) -> String>,
    /// "3 jobs finished · failures".
    pub with_failures: Rc<dyn Fn(&str) -> String>,
    /// "2 running" (the dashboard's summary).
    pub running: Rc<dyn Fn(usize) -> String>,
    /// "3 finished".
    pub finished: Rc<dyn Fn(usize) -> String>,
    /// "Running for 12s", "Done in 1m 5s", …
    pub status: StatusPhrase,
    /// "4 failed".
    pub failed: Rc<dyn Fn(usize) -> String>,
    /// "2 older jobs are hidden".
    pub hidden: Rc<dyn Fn(usize) -> String>,
    /// A duration in whole seconds.
    pub duration: Rc<dyn Fn(u64) -> String>,
    pub integer: Rc<dyn Fn(u64) -> String>,
    pub title: String,
    pub show: String,
    pub none: String,
    pub no_output: String,
    pub truncated: String,
    pub clear: String,
    pub minimize: String,
    pub cancel: String,
    pub remove: String,
    pub details: String,
    pub hide: String,
    pub dismiss: String,
}

impl fmt::Debug for JobsWords {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobsWords").finish_non_exhaustive()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        format::integer(n as u64),
        if n == 1 { one } else { many }
    )
}

impl Default for JobsWords {
    fn default() -> Self {
        JobsWords {
            jobs_running: Rc::new(|n| plural(n, "job running", "jobs running")),
            queued: Rc::new(|n| format!("{} queued", format::integer(n as u64))),
            jobs_finished: Rc::new(|n| plural(n, "job finished", "jobs finished")),
            with_failures: Rc::new(|jobs| format!("{jobs} · failures")),
            running: Rc::new(|n| format!("{} running", format::integer(n as u64))),
            finished: Rc::new(|n| format!("{} finished", format::integer(n as u64))),
            status: Rc::new(|status, elapsed| match status {
                JobStatus::Queued => "Queued".into(),
                JobStatus::Running => format!("Running for {elapsed}"),
                JobStatus::Cancelling => "Cancelling…".into(),
                JobStatus::Succeeded => format!("Done in {elapsed}"),
                JobStatus::Failed => format!("Failed after {elapsed}"),
                JobStatus::Cancelled => format!("Cancelled after {elapsed}"),
            }),
            failed: Rc::new(|n| format!("{} failed", format::integer(n as u64))),
            hidden: Rc::new(|n| format!("{} older jobs are hidden", format::integer(n as u64))),
            duration: Rc::new(format::duration),
            integer: Rc::new(format::integer),
            title: "Jobs".into(),
            show: "Show background jobs".into(),
            none: "No background jobs.".into(),
            no_output: "No output yet.".into(),
            truncated: "Older output was discarded to bound memory".into(),
            clear: "Clear finished".into(),
            minimize: "Minimize".into(),
            cancel: "Cancel".into(),
            remove: "Remove".into(),
            details: "Details".into(),
            hide: "Hide".into(),
            dismiss: "Dismiss".into(),
        }
    }
}

/// The footer button's words, or `None` with no jobs: "2 jobs running · 1 queued", else
/// "3 jobs finished", with "· failures" when one failed (Strata's `indicator_label`).
pub fn indicator(rows: &[JobRow], words: &JobsWords) -> Option<String> {
    let running = rows.iter().filter(|r| r.status.is_running()).count();
    let queued = rows
        .iter()
        .filter(|r| r.status == JobStatus::Queued)
        .count();
    let mut parts = Vec::new();
    if running > 0 {
        parts.push((words.jobs_running)(running));
    }
    if queued > 0 {
        parts.push((words.queued)(queued));
    }
    if !parts.is_empty() {
        return Some(parts.join(" · "));
    }
    let finished = rows.iter().filter(|r| r.status.is_finished()).count();
    if finished == 0 {
        return None;
    }
    let jobs = (words.jobs_finished)(finished);
    Some(if rows.iter().any(|r| r.status == JobStatus::Failed) {
        (words.with_failures)(&jobs)
    } else {
        jobs
    })
}

/// The dashboard's summary: "2 running · 1 queued", and the finished count ("3 finished"),
/// which the list draws after a "· " when both show.
pub(crate) fn activity(rows: &[JobRow], words: &JobsWords) -> (Option<String>, Option<String>) {
    let running = rows.iter().filter(|r| r.status.is_running()).count();
    let queued = rows
        .iter()
        .filter(|r| r.status == JobStatus::Queued)
        .count();
    let finished = rows.iter().filter(|r| r.status.is_finished()).count();
    let active = match (running, queued) {
        (0, 0) => None,
        (0, q) => Some((words.queued)(q)),
        (r, 0) => Some((words.running)(r)),
        (r, q) => Some(format!("{} · {}", (words.running)(r), (words.queued)(q))),
    };
    let done = (finished > 0).then(|| (words.finished)(finished));
    (active, done)
}

/// A row's status with its elapsed time, at `now` on the host's clock.
pub(crate) fn status(row: &JobRow, now: f64, words: &JobsWords) -> String {
    let end = row.ended_at.unwrap_or(now);
    let elapsed = (end - row.started_at).max(0.0) as u64;
    (words.status)(row.status, &(words.duration)(elapsed))
}

/// A row's meta line: its message, its item count and failures, and its folder (Strata's
/// `meta_label`).
pub(crate) fn meta(row: &JobRow, words: &JobsWords) -> String {
    let mut parts = Vec::new();
    if let Some(m) = &row.message {
        parts.push(m.clone());
    }
    if let Some((done, total)) = row.items.filter(|&(_, t)| t > 0) {
        parts.push(format!(
            "{}/{}",
            (words.integer)(done as u64),
            (words.integer)(total as u64)
        ));
        if row.failed > 0 {
            parts.push((words.failed)(row.failed));
        }
    }
    if !row.folder.is_empty() {
        parts.push(row.folder.clone());
    }
    parts.join(" · ")
}

/// What Details shows: the error first unless the output already says it, then the output
/// with terminal sequences stripped, or "No output yet." (Strata's `details_text`).
pub(crate) fn details(row: &JobRow, words: &JobsWords) -> String {
    let log = strip_terminal_sequences(row.output.trim_end());
    let text = match row
        .error
        .as_deref()
        .filter(|_| row.status != JobStatus::Succeeded)
    {
        Some(m) if !log.contains(m) => format!("{m}\n{log}").trim_end().to_owned(),
        _ => log,
    };
    if text.is_empty() {
        words.no_output.clone()
    } else {
        text
    }
}

/// A per-item job's fraction: completed items plus the current item's reported fraction, over
/// the total (Strata's per-item progress).
pub fn per_item_fraction(done: usize, total: usize, current: Option<f32>) -> Option<f32> {
    (total > 0).then(|| {
        ((done as f32 + current.unwrap_or(0.0).clamp(0.0, 1.0)) / total as f32).clamp(0.0, 1.0)
    })
}

/// `text` without terminal colour and control sequences: CSI (`ESC [ … final`), OSC
/// (`ESC ] … BEL` or `ESC \`), other two-character escapes, and control characters other
/// than newline and tab (Strata's `strip_terminal_sequences`).
pub fn strip_terminal_sequences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.next() == Some('\\') {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}
