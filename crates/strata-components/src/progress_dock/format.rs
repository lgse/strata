//! Byte sizes, rates, durations and percentages as Strata writes them in English
//! (lgse/strata `src/i18n.rs`: `file_size`, `transfer_rate`, `duration`, `percent`, `integer`).
//! Hosts pass their own through [`Words`](super::Words) to translate them.

/// Strata's size units: decimal, as GLib and GNOME write file sizes.
const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];

/// A byte count in decimal units with at most one fraction digit, shown only when it isn't
/// zero: "999 B", "1.5 kB", "64 MB", "1 GB" (Strata's `i18n::file_size`). A value that rounds
/// to 1000 of a unit moves to the next: 999,960 bytes is "1 MB", not "1000 kB".
pub fn size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} {}", UNITS[0]);
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    let mut rounded = (value * 10.0).round() / 10.0;
    if rounded >= 1000.0 && unit + 1 < UNITS.len() {
        rounded /= 1000.0;
        unit += 1;
    }
    if rounded.fract() == 0.0 {
        format!("{} {}", integer(rounded as u64), UNITS[unit])
    } else {
        let whole = rounded.trunc() as u64;
        let tenth = ((rounded - rounded.trunc()) * 10.0).round() as u64;
        format!("{}.{tenth} {}", integer(whole), UNITS[unit])
    }
}

/// A byte rate: "1.5 MB/s" (Strata's `i18n::transfer_rate`).
pub fn rate(bytes_per_second: u64) -> String {
    format!("{}/s", size(bytes_per_second))
}

/// A compact whole-second duration: "45s", "2m 5s", "1h 3m" (Strata's `i18n::duration`).
pub fn duration(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h {}m", integer(seconds / 3600), seconds % 3600 / 60)
    }
}

/// A whole number with its thousands grouped: "270,325".
pub fn integer(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A whole percentage: "42%".
pub fn percent(value: u32) -> String {
    format!("{value}%")
}

/// The whole percentage a fraction shows, rounded down, as Strata's bar label rounds it. Once
/// anything has moved (`started`), it shows at least 1 %, so a long copy never reads 0 %
/// while it runs.
pub fn whole_percent(fraction: f64, started: bool) -> u32 {
    let p = (fraction.clamp(0.0, 1.0) * 100.0) as u32;
    if started { p.max(1) } else { p }
}

/// The seconds left at `rate` bytes a second for `remaining` bytes, rounded up and at least 1
/// (Strata's `transfer_rate_status`). `None` with no rate measured yet, or nothing left.
pub fn seconds_left(rate: Option<f64>, remaining: Option<u64>) -> Option<u64> {
    let rate = rate.filter(|r| r.is_finite() && *r > 0.0)?;
    let remaining = remaining.filter(|&r| r > 0)?;
    Some((remaining as f64 / rate).ceil().max(1.0) as u64)
}
