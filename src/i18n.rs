// SPDX-License-Identifier: MIT

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// The saved choice is separate from the process locale: changes apply on restart.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) enum Language {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "ja")]
    Japanese,
    #[serde(rename = "pt-BR")]
    BrazilianPortuguese,
    #[serde(rename = "ko")]
    Korean,
    #[serde(rename = "vi")]
    Vietnamese,
    #[serde(rename = "it")]
    Italian,
    #[serde(rename = "ru")]
    Russian,
    #[default]
    #[serde(other, rename = "auto")]
    Auto,
}

impl Language {
    pub const CHOICES: [(&'static str, Self); 11] = [
        ("Auto-detect", Self::Auto),
        ("English", Self::English),
        ("Français", Self::French),
        ("Deutsch", Self::German),
        ("Español", Self::Spanish),
        ("日本語", Self::Japanese),
        ("Português (Brasil)", Self::BrazilianPortuguese),
        ("한국어", Self::Korean),
        ("Tiếng Việt", Self::Vietnamese),
        ("Italiano", Self::Italian),
        ("Русский", Self::Russian),
    ];

    pub fn locale(self) -> &'static str {
        match self {
            Self::Auto => detected_locale(),
            Self::English => "en",
            Self::French => "fr",
            Self::German => "de",
            Self::Spanish => "es",
            Self::Japanese => "ja",
            Self::BrazilianPortuguese => "pt-BR",
            Self::Korean => "ko",
            Self::Vietnamese => "vi",
            Self::Italian => "it",
            Self::Russian => "ru",
        }
    }
}

fn supported_locale(value: &str) -> Option<&'static str> {
    let value = value
        .trim()
        .split(['.', '@'])
        .next()?
        .replace('_', "-")
        .to_ascii_lowercase();
    match value.split('-').next()? {
        "en" | "c" | "posix" => Some("en"),
        "fr" => Some("fr"),
        "de" => Some("de"),
        "es" => Some("es"),
        "ja" => Some("ja"),
        "pt" => Some("pt-BR"),
        "ko" => Some("ko"),
        "vi" => Some("vi"),
        "it" => Some("it"),
        "ru" => Some("ru"),
        _ => None,
    }
}

fn resolve_locale(language: &str, lc_all: &str, lc_messages: &str, lang: &str) -> &'static str {
    let messages = [lc_all, lc_messages, lang]
        .into_iter()
        .find(|value| !value.is_empty())
        .unwrap_or("C");
    // GNU LANGUAGE is a priority list, but the explicit POSIX locale suppresses it.
    if matches!(messages, "C" | "POSIX") {
        return "en";
    }
    language
        .split(':')
        .chain(std::iter::once(messages))
        .find_map(supported_locale)
        .unwrap_or("en")
}

pub(crate) fn detected_locale() -> &'static str {
    let get = |name| std::env::var(name).unwrap_or_default();
    resolve_locale(
        &get("LANGUAGE"),
        &get("LC_ALL"),
        &get("LC_MESSAGES"),
        &get("LANG"),
    )
}

mod compiled {
    include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));
}

/// Translation backend over the tables that build.rs generates from `data/locales`.
pub(crate) struct Catalogs(HashMap<&'static str, HashMap<&'static str, &'static str>>);

impl Catalogs {
    pub(crate) fn compiled() -> Self {
        Self(
            compiled::CATALOGS
                .iter()
                .map(|&(locale, messages)| (locale, messages.iter().copied().collect()))
                .collect(),
        )
    }
}

impl rust_i18n::Backend for Catalogs {
    fn available_locales(&self) -> Vec<&str> {
        let mut locales: Vec<&str> = self.0.keys().copied().collect();
        locales.sort_unstable();
        locales
    }

    fn translate(&self, locale: &str, key: &str) -> Option<&str> {
        self.0.get(locale)?.get(key).copied()
    }
}

/// English source messages are keys; missing translations retain readable English.
/// Call only at presentation boundaries, never for paths, protocol values or user content.
pub(crate) fn tr(message: &str) -> String {
    rust_i18n::t!(message).into_owned()
}

/// Joins already translated items with the language's list separator.
pub(crate) fn list(items: impl IntoIterator<Item = String>) -> String {
    list_in(&rust_i18n::locale(), items)
}

/// Uppercases the first character, for lower-case list fragments that start a sentence.
pub(crate) fn capitalize_first(text: String) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if !first.is_uppercase() => first.to_uppercase().chain(chars).collect(),
        _ => text,
    }
}

fn list_in(locale: &str, items: impl IntoIterator<Item = String>) -> String {
    items
        .into_iter()
        .reduce(|first, second| {
            rust_i18n::t!(
                "%{first}, %{second}",
                locale = locale,
                first = first,
                second = second
            )
            .into_owned()
        })
        .unwrap_or_default()
}

fn plural_category(locale: &str, count: u64) -> &'static str {
    match locale {
        "ja" | "ko" | "vi" => "other",
        "fr" | "pt-BR" if count <= 1 => "one",
        "ru" if count % 10 == 1 && count % 100 != 11 => "one",
        "ru" if (2..=4).contains(&(count % 10)) && !(12..=14).contains(&(count % 100)) => "few",
        "ru" => "many",
        _ if count == 1 => "one",
        _ => "other",
    }
}

pub(crate) fn count(kind: &str, count: usize) -> String {
    count_u64(kind, count as u64)
}

pub(crate) fn count_u64(kind: &str, count: u64) -> String {
    count_in(&rust_i18n::locale(), kind, count)
}

fn count_in(locale: &str, kind: &str, count: u64) -> String {
    let category = plural_category(locale, count);
    rust_i18n::t!(
        &format!("counts.{kind}.{category}"),
        locale = locale,
        count = integer_in(locale, count)
    )
    .into_owned()
}

fn decimal_separator(locale: &str) -> &'static str {
    match locale {
        "en" | "ja" | "ko" => ".",
        _ => ",",
    }
}

fn group_separator(locale: &str) -> &'static str {
    match locale {
        "en" | "ja" | "ko" => ",",
        "fr" => "\u{202f}",
        "ru" => "\u{a0}",
        _ => ".",
    }
}

fn group_digits(locale: &str, digits: &str) -> String {
    // Spanish leaves four-digit numbers ungrouped.
    let minimum = if locale == "es" { 5 } else { 4 };
    if digits.len() < minimum {
        return digits.to_owned();
    }
    let separator = group_separator(locale);
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3 * separator.len());
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push_str(separator);
        }
        grouped.push(digit);
    }
    grouped
}

/// Formats a whole number with the language's digit grouping.
pub(crate) fn integer(value: u64) -> String {
    integer_in(&rust_i18n::locale(), value)
}

fn integer_in(locale: &str, value: u64) -> String {
    group_digits(locale, &value.to_string())
}

/// A whole percentage such as "42%", spaced as the language writes it.
pub(crate) fn percent(value: usize) -> String {
    percent_in(&rust_i18n::locale(), value)
}

fn percent_in(locale: &str, value: usize) -> String {
    rust_i18n::t!(
        "%{value}%",
        locale = locale,
        value = integer_in(locale, value as u64)
    )
    .into_owned()
}

/// Formats `value` with exactly `decimals` fraction digits in the language's notation.
pub(crate) fn decimal(value: f64, decimals: usize) -> String {
    decimal_in(&rust_i18n::locale(), value, decimals)
}

fn decimal_in(locale: &str, value: f64, decimals: usize) -> String {
    let text = format!("{:.*}", decimals, value.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let mut formatted = String::new();
    if value < 0.0 {
        formatted.push('-');
    }
    formatted.push_str(&group_digits(locale, whole));
    if !fraction.is_empty() {
        formatted.push_str(decimal_separator(locale));
        formatted.push_str(fraction);
    }
    formatted
}

const SIZE_UNITS: [&str; 5] = [
    "%{size} B",
    "%{size} kB",
    "%{size} MB",
    "%{size} GB",
    "%{size} TB",
];

/// Divides `bytes` into the largest decimal unit whose threshold it meets after
/// rounding to one decimal, returning the rounded value and the unit index.
fn rounded_size_and_unit(bytes: u64, unit_count: usize) -> (f64, usize) {
    if bytes < 1_000 {
        return (bytes as f64, 0);
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1_000.0 && unit + 1 < unit_count {
        value /= 1_000.0;
        unit += 1;
    }
    let rounded = (value * 10.0).round() / 10.0;
    if rounded >= 1_000.0 && unit + 1 < unit_count {
        (rounded / 1_000.0, unit + 1)
    } else {
        (rounded, unit)
    }
}

/// The one shared byte-size presentation: decimal units, at most one fraction
/// digit, and the language's decimal separator and unit symbols.
pub(crate) fn file_size(bytes: u64) -> String {
    file_size_in(&rust_i18n::locale(), bytes)
}

fn file_size_in(locale: &str, bytes: u64) -> String {
    let (value, unit) = rounded_size_and_unit(bytes, SIZE_UNITS.len());
    let decimals = usize::from(value.fract() != 0.0);
    rust_i18n::t!(
        SIZE_UNITS[unit],
        locale = locale,
        size = decimal_in(locale, value, decimals)
    )
    .into_owned()
}

/// A byte rate such as "1.5 MB/s".
pub(crate) fn transfer_rate(bytes_per_second: u64) -> String {
    transfer_rate_in(&rust_i18n::locale(), bytes_per_second)
}

fn transfer_rate_in(locale: &str, bytes_per_second: u64) -> String {
    rust_i18n::t!(
        "%{size}/s",
        locale = locale,
        size = file_size_in(locale, bytes_per_second)
    )
    .into_owned()
}

/// A compact whole-second duration such as "45s", "2m 5s" or "1h 3m".
pub(crate) fn duration(seconds: u64) -> String {
    duration_in(&rust_i18n::locale(), seconds)
}

fn duration_in(locale: &str, seconds: u64) -> String {
    let text = if seconds < 60 {
        rust_i18n::t!("%{seconds}s", locale = locale, seconds = seconds)
    } else if seconds < 3_600 {
        rust_i18n::t!(
            "%{minutes}m %{seconds}s",
            locale = locale,
            minutes = seconds / 60,
            seconds = seconds % 60
        )
    } else {
        rust_i18n::t!(
            "%{hours}h %{minutes}m",
            locale = locale,
            hours = integer_in(locale, seconds / 3_600),
            minutes = seconds % 3_600 / 60
        )
    };
    text.into_owned()
}

#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod tests;
