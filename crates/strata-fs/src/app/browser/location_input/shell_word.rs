// SPDX-License-Identifier: MIT

use std::iter::Peekable;
use std::str::Chars;

pub(super) fn outer_quotes_removed(input: &str) -> Option<&str> {
    ['\'', '"']
        .into_iter()
        .find_map(|quote| input.strip_prefix(quote)?.strip_suffix(quote))
}

/// The bytes `input` stands for as one shell word built from backslash escapes and
/// `'…'`, `"…"` and `$'…'` quoting, or `None` when it is not one: such text is a
/// literal path, so a backslash before a character no shell quoting escapes stays.
pub(super) fn unquoted(input: &str) -> Option<Vec<u8>> {
    if !input.contains(['\\', '\'', '"']) {
        return None;
    }
    let mut word = Vec::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    // Shells and terminals open a quote at the start of a path component or next to
    // other quoting, so one inside a name, as in `Mom's and Dad's`, is an apostrophe.
    let mut inside_name = false;
    while let Some(character) = chars.next() {
        if let Some(quoted) = quote_opened_by(character, &mut chars) {
            if inside_name {
                return None;
            }
            quoted(&mut chars, &mut word)?;
            inside_name = false;
            continue;
        }
        match character {
            '\\' => push(&mut word, chars.next().filter(|&next| escapable(next))?),
            character if splits_word(character) => return None,
            character => push(&mut word, character),
        }
        inside_name = !matches!(character, '\\' | '/');
    }
    // No path holds a NUL, and glib-rs panics converting one.
    (!word.contains(&0)).then_some(word)
}

type Quoted = fn(&mut Peekable<Chars<'_>>, &mut Vec<u8>) -> Option<()>;

fn quote_opened_by(character: char, chars: &mut Peekable<Chars<'_>>) -> Option<Quoted> {
    match character {
        '\'' => Some(single_quoted),
        '"' => Some(double_quoted),
        '$' if chars.next_if_eq(&'\'').is_some() => Some(ansi_c_quoted),
        _ => None,
    }
}

/// Characters that `printf %q`, shell completion and the former Copy path escape.
fn escapable(character: char) -> bool {
    character.is_whitespace()
        || character.is_control()
        || matches!(
            character,
            '"' | '\''
                | '\\'
                | '$'
                | '`'
                | '!'
                | '#'
                | '&'
                | '*'
                | ';'
                | '<'
                | '>'
                | '?'
                | '['
                | ']'
                | '{'
                | '}'
                | '('
                | ')'
                | '|'
                | '~'
                | ','
                | '^'
                | '='
        )
}

/// Unquoted, these would end the word or expand in a shell, so the text was not
/// written as one quoted path.
fn splits_word(character: char) -> bool {
    character.is_whitespace()
        || character.is_control()
        || matches!(
            character,
            '$' | '`' | '&' | ';' | '<' | '>' | '(' | ')' | '|'
        )
}

fn single_quoted(chars: &mut Peekable<Chars<'_>>, word: &mut Vec<u8>) -> Option<()> {
    for character in chars.by_ref() {
        if character == '\'' {
            return Some(());
        }
        push(word, character);
    }
    None
}

fn double_quoted(chars: &mut Peekable<Chars<'_>>, word: &mut Vec<u8>) -> Option<()> {
    while let Some(character) = chars.next() {
        match character {
            '"' => return Some(()),
            '\\' => {
                if let Some(escaped) =
                    chars.next_if(|next| matches!(next, '$' | '`' | '"' | '\\' | '\n'))
                {
                    push(word, escaped);
                } else {
                    push(word, '\\');
                }
            }
            character => push(word, character),
        }
    }
    None
}

fn ansi_c_quoted(chars: &mut Peekable<Chars<'_>>, word: &mut Vec<u8>) -> Option<()> {
    while let Some(character) = chars.next() {
        match character {
            '\'' => return Some(()),
            '\\' => ansi_c_escape(chars.next()?, chars, word),
            character => push(word, character),
        }
    }
    None
}

fn ansi_c_escape(escape: char, chars: &mut Peekable<Chars<'_>>, word: &mut Vec<u8>) {
    let byte = match escape {
        'a' => 0x07,
        'b' => 0x08,
        'e' | 'E' => 0x1b,
        'f' => 0x0c,
        'n' => b'\n',
        'r' => b'\r',
        't' => b'\t',
        'v' => 0x0b,
        '\\' | '\'' | '"' | '?' => escape as u8,
        '0'..='7' => digits(chars, 8, 2, escape.to_digit(8).unwrap_or_default()) as u8,
        'x' | 'u' | 'U' => return numeric_escape(escape, chars, word),
        _ => {
            push(word, '\\');
            return push(word, escape);
        }
    };
    word.push(byte);
}

/// `\xHH` is a raw byte, so it can spell a name that is not UTF-8; `\u` and `\U`
/// are code points. Without digits the escape stays literal, as in Bash.
fn numeric_escape(escape: char, chars: &mut Peekable<Chars<'_>>, word: &mut Vec<u8>) {
    let max = match escape {
        'x' => 2,
        'u' => 4,
        _ => 8,
    };
    if !chars.peek().is_some_and(char::is_ascii_hexdigit) {
        push(word, '\\');
        return push(word, escape);
    }
    let value = digits(chars, 16, max, 0);
    match (escape, char::from_u32(value)) {
        ('x', _) => word.push(value as u8),
        (_, Some(character)) => push(word, character),
        (_, None) => {}
    }
}

fn digits(chars: &mut Peekable<Chars<'_>>, radix: u32, max: usize, mut value: u32) -> u32 {
    for _ in 0..max {
        let Some(digit) = chars.peek().and_then(|next| next.to_digit(radix)) else {
            break;
        };
        chars.next();
        value = value * radix + digit;
    }
    value
}

fn push(word: &mut Vec<u8>, character: char) {
    word.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
}
