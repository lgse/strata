// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn detects_supported_locales_and_honors_posix_precedence() {
    for (language, all, messages, lang, expected) in [
        ("", "", "", "fr_CA.UTF-8", "fr"),
        ("", "de_DE.UTF-8", "fr_FR", "ja_JP", "de"),
        ("", "", "ko_KR", "en_US", "ko"),
        ("xx:ru_RU:fr", "", "", "de_DE", "ru"),
        ("xx:zz", "", "", "vi_VN", "vi"),
        ("fr", "C", "", "de_DE", "en"),
        ("fr", "POSIX", "", "de_DE", "en"),
        ("ja", "", "", "C.UTF-8", "ja"),
        ("", "", "", "pt_BR.UTF-8", "pt-BR"),
        ("", "", "", "pt-PT", "pt-BR"),
        ("", "", "", "es_MX@variant", "es"),
        ("", "", "", "IT_it.utf8", "it"),
        ("", "", "", "unsupported", "en"),
        ("", "", "", "", "en"),
    ] {
        assert_eq!(
            resolve_locale(language, all, messages, lang),
            expected,
            "{language}/{all}/{messages}/{lang}"
        );
    }
}

#[test]
fn manual_language_does_not_depend_on_system_locale() {
    for (choice, locale) in [
        (Language::English, "en"),
        (Language::French, "fr"),
        (Language::German, "de"),
        (Language::Spanish, "es"),
        (Language::Japanese, "ja"),
        (Language::BrazilianPortuguese, "pt-BR"),
        (Language::Korean, "ko"),
        (Language::Vietnamese, "vi"),
        (Language::Italian, "it"),
        (Language::Russian, "ru"),
    ] {
        assert_eq!(choice.locale(), locale);
        assert_eq!(
            serde_json::from_str::<Language>(
                &serde_json::to_string(&choice).expect("serialize language")
            )
            .expect("deserialize language"),
            choice
        );
    }
    assert_eq!(
        serde_json::from_str::<Language>("\"future-language\"").expect("recover unknown language"),
        Language::Auto
    );
}

#[test]
fn plural_categories_cover_russian_teens_and_invariant_asian_forms() {
    for (locale, values) in [
        (
            "ru",
            vec![
                (0, "many"),
                (1, "one"),
                (2, "few"),
                (5, "many"),
                (11, "many"),
                (12, "many"),
                (21, "one"),
                (22, "few"),
                (25, "many"),
                (111, "many"),
            ],
        ),
        ("fr", vec![(0, "one"), (1, "one"), (2, "other")]),
        ("pt-BR", vec![(0, "one"), (1, "one"), (2, "other")]),
        ("en", vec![(0, "other"), (1, "one"), (2, "other")]),
        ("ja", vec![(0, "other"), (1, "other"), (2, "other")]),
        ("ko", vec![(1, "other")]),
        ("vi", vec![(1, "other")]),
    ] {
        for (count, expected) in values {
            assert_eq!(plural_category(locale, count), expected, "{locale}/{count}");
        }
    }
    assert_eq!(
        rust_i18n::t!("counts.items.few", locale = "ru", count = 22),
        "22 элемента"
    );
    assert_eq!(
        rust_i18n::t!("counts.files.other", locale = "ja", count = 3),
        "3個のファイル"
    );
}

#[test]
fn lists_use_the_language_separator() {
    let items = || ["2 folders", "3 files", "1 link"].map(str::to_owned);
    assert_eq!(list_in("en", items()), "2 folders, 3 files, 1 link");
    assert_eq!(list_in("ja", items()), "2 folders、3 files、1 link");
    assert_eq!(list_in("en", []), "");
    assert_eq!(
        capitalize_first("с ошибкой: 1 элемент, не обработано: 0 элементов".to_owned()),
        "С ошибкой: 1 элемент, не обработано: 0 элементов"
    );
    assert_eq!(
        capitalize_first("thất bại 2 mục".to_owned()),
        "Thất bại 2 mục"
    );
    assert_eq!(capitalize_first("0 件完了".to_owned()), "0 件完了");
}

#[test]
fn interpolation_preserves_user_text_without_retranslating_or_expanding_it() {
    let filename = "Language %{value2} <&> 日本語.txt";
    assert_eq!(
        rust_i18n::t!(
            "This runs “%{value1}” on %{value2}.",
            locale = "en",
            value1 = filename,
            value2 = "1 file"
        ),
        "This runs “Language %{value2} <&> 日本語.txt” on 1 file.",
    );
}

#[test]
fn english_fallback_keeps_missing_messages_readable() {
    assert_eq!(
        rust_i18n::t!("Language", locale = "unsupported"),
        "Language"
    );
    assert_eq!(
        rust_i18n::t!("A message not yet translated", locale = "unsupported"),
        "A message not yet translated"
    );
}

#[test]
fn file_sizes_use_the_language_decimal_separator_and_unit_symbols() {
    for (locale, bytes, expected) in [
        ("en", 999, "999 B"),
        ("en", 1_500, "1.5 kB"),
        ("en", 2_000, "2 kB"),
        ("en", 999_960, "1 MB"),
        ("de", 1_500, "1,5 kB"),
        ("de", 30_800_000_000, "30,8 GB"),
        ("fr", 6, "6 o"),
        ("fr", 1_500, "1,5 ko"),
        ("fr", 30_800_000_000, "30,8 Go"),
        ("ru", 37, "37 Б"),
        ("ru", 1_500, "1,5 кБ"),
        ("ru", 12_300_000, "12,3 МБ"),
        ("ja", 1_500, "1.5 kB"),
    ] {
        assert_eq!(file_size_in(locale, bytes), expected, "{locale}/{bytes}");
    }
    assert_eq!(transfer_rate_in("en", 1_000), "1 kB/s");
    assert_eq!(transfer_rate_in("ru", 128_500_000), "128,5 МБ/с");
}

#[test]
fn counts_and_numbers_group_digits_by_language() {
    assert_eq!(count_in("en", "files", 3_000), "3,000 files");
    assert_eq!(count_in("en", "files", 999), "999 files");
    assert_eq!(count_in("de", "files", 3_000), "3.000 Dateien");
    assert_eq!(count_in("es", "files", 3_000), "3000 archivos");
    assert_eq!(count_in("es", "files", 60_000), "60.000 archivos");
    assert_eq!(count_in("fr", "files", 60_000), "60\u{202f}000 fichiers");
    assert_eq!(count_in("ru", "files", 60_000), "60\u{a0}000 файлов");
    assert_eq!(decimal_in("de", 1_234_567.3, 1), "1.234.567,3");
    assert_eq!(decimal_in("en", 400.0, 1), "400.0");
}

#[test]
fn durations_use_the_language_units() {
    for (locale, seconds, expected) in [
        ("en", 0, "0s"),
        ("en", 125, "2m 5s"),
        ("en", 3_725, "1h 2m"),
        ("de", 2, "2 s"),
        ("ru", 125, "2 мин 5 с"),
        ("ja", 3_725, "1時間2分"),
    ] {
        assert_eq!(duration_in(locale, seconds), expected, "{locale}/{seconds}");
    }
}
