// SPDX-License-Identifier: MIT

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

/// English source messages are keys; missing translations retain readable English.
/// Call only at presentation boundaries, never for paths, protocol values or user content.
pub(crate) fn tr(message: &str) -> String {
    rust_i18n::t!(message).into_owned()
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
    let locale = rust_i18n::locale();
    let category = plural_category(&locale, count);
    rust_i18n::t!(&format!("counts.{kind}.{category}"), count = count).into_owned()
}

#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod tests;
