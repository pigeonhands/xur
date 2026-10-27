//! Decoding of escape sequences in string and byte-string literals, after
//! `rustc_lexer::unescape`.
//!
//! The decoder reports each character or error together with the byte range
//! of the literal's contents it came from, so errors can point at the exact
//! escape sequence.

use std::ops::Range;
use std::str::Chars;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum EscapeError {
    #[error("unknown character escape: `{0}`")]
    UnknownEscape(char),
    #[error("incomplete escape sequence")]
    LoneBackslash,
    #[error("incorrect unicode escape sequence")]
    NoBraceInUnicodeEscape,
    #[error("unterminated unicode escape")]
    UnclosedUnicodeEscape,
    #[error("empty unicode escape")]
    EmptyUnicodeEscape,
    #[error("invalid character in unicode escape: `{0}`")]
    InvalidCharInUnicodeEscape(char),
    #[error("overlong unicode escape")]
    OverlongUnicodeEscape,
    #[error("invalid unicode character escape")]
    InvalidUnicodeEscape,
    #[error("numeric character escape is too short")]
    TooShortHexEscape,
    #[error("invalid character in numeric character escape: `{0}`")]
    InvalidCharInHexEscape(char),
    #[error("out of range hex escape")]
    OutOfRangeHexEscape,
    #[error("non-ASCII character in byte string literal")]
    NonAsciiInByteString(char),
    #[error("unicode escape in byte string")]
    UnicodeEscapeInByteString,
}

impl EscapeError {
    /// A short explanation to attach to the offending escape.
    pub fn label(self) -> &'static str {
        match self {
            Self::UnknownEscape(_) => "unknown character escape",
            Self::LoneBackslash => "expected an escape character after `\\`",
            Self::NoBraceInUnicodeEscape => "expected `{` after `\\u`, as in `\\u{1F600}`",
            Self::UnclosedUnicodeEscape => "missing closing `}`",
            Self::EmptyUnicodeEscape => "this escape must have at least 1 hex digit",
            Self::InvalidCharInUnicodeEscape(_) => "not a hexadecimal digit",
            Self::OverlongUnicodeEscape => "must have at most 6 hex digits",
            Self::InvalidUnicodeEscape => "not a valid unicode scalar value",
            Self::TooShortHexEscape => "a hex escape takes two digits, as in `\\x7F`",
            Self::InvalidCharInHexEscape(_) => "not a hexadecimal digit",
            Self::OutOfRangeHexEscape => "must be at most `\\x7F` in a string",
            Self::NonAsciiInByteString(_) => "write non-ASCII bytes as hex escapes, like `\\xFF`",
            Self::UnicodeEscapeInByteString => "write bytes as hex escapes, like `\\xFF`",
        }
    }
}

/// Decodes the contents of a string literal (without the quotes), calling
/// `callback` with the byte range and result of each character or escape.
pub fn unescape_str(src: &str, mut callback: impl FnMut(Range<usize>, Result<char, EscapeError>)) {
    unescape(src, Mode::Str, |range, result| {
        let result = result.map(|code| char::from_u32(code).expect("decoded a valid char"));
        callback(range, result);
    });
}

/// Like [`unescape_str`], for the contents of a byte string literal: each
/// result is one byte.
pub fn unescape_bytes(src: &str, mut callback: impl FnMut(Range<usize>, Result<u8, EscapeError>)) {
    unescape(src, Mode::Bytes, |range, result| {
        let result = result.map(|code| u8::try_from(code).expect("decoded a byte"));
        callback(range, result);
    });
}

/// What a literal's characters and escapes decode to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Characters, for `"..."`.
    Str,
    /// Bytes, for `b"..."`: ASCII characters and escapes up to `\xFF`.
    Bytes,
}

/// Decodes `src`, calling `callback` with each character's code point (in
/// [`Mode::Str`]) or byte value (in [`Mode::Bytes`]).
fn unescape(
    src: &str,
    mode: Mode,
    mut callback: impl FnMut(Range<usize>, Result<u32, EscapeError>),
) {
    let offset = |rest: &Chars<'_>| src.len() - rest.as_str().len();
    let mut chars = src.chars();
    loop {
        let start = offset(&chars);
        let result = match chars.next() {
            None => return,
            Some('\\') => scan_escape(&mut chars, mode),
            Some(c) if mode == Mode::Bytes && !c.is_ascii() => {
                Err(EscapeError::NonAsciiInByteString(c))
            }
            Some(c) => Ok(c.into()),
        };
        callback(start..offset(&chars), result);
    }
}

/// Decodes an escape whose `\` was already consumed.
fn scan_escape(chars: &mut Chars<'_>, mode: Mode) -> Result<u32, EscapeError> {
    let c = match chars.next().ok_or(EscapeError::LoneBackslash)? {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        '0' => '\0',
        '\\' => '\\',
        '"' => '"',
        '\'' => '\'',
        'x' => return scan_hex(chars, mode),
        'u' if mode == Mode::Bytes => return Err(EscapeError::UnicodeEscapeInByteString),
        'u' => return scan_unicode(chars).map(u32::from),
        other => return Err(EscapeError::UnknownEscape(other)),
    };
    Ok(c.into())
}

/// Decodes the two hex digits after `\x`.
fn scan_hex(chars: &mut Chars<'_>, mode: Mode) -> Result<u32, EscapeError> {
    let mut value = 0;
    for _ in 0..2 {
        let c = chars.next().ok_or(EscapeError::TooShortHexEscape)?;
        let digit = c
            .to_digit(16)
            .ok_or(EscapeError::InvalidCharInHexEscape(c))?;
        value = value * 16 + digit;
    }
    if mode == Mode::Str && value > 0x7F {
        return Err(EscapeError::OutOfRangeHexEscape);
    }
    Ok(value)
}

/// Decodes `{XXXX}` after `\u`.
fn scan_unicode(chars: &mut Chars<'_>) -> Result<char, EscapeError> {
    if chars.next() != Some('{') {
        return Err(EscapeError::NoBraceInUnicodeEscape);
    }
    let mut value = 0u32;
    let mut digits = 0;
    loop {
        match chars.next() {
            None => return Err(EscapeError::UnclosedUnicodeEscape),
            Some('}') => break,
            Some(c) => {
                let digit = c
                    .to_digit(16)
                    .ok_or(EscapeError::InvalidCharInUnicodeEscape(c))?;
                digits += 1;
                if digits > 6 {
                    return Err(EscapeError::OverlongUnicodeEscape);
                }
                value = value * 16 + digit;
            }
        }
    }
    if digits == 0 {
        return Err(EscapeError::EmptyUnicodeEscape);
    }
    char::from_u32(value).ok_or(EscapeError::InvalidUnicodeEscape)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unescape(src: &str) -> Vec<(Range<usize>, Result<char, EscapeError>)> {
        let mut out = Vec::new();
        unescape_str(src, |range, result| out.push((range, result)));
        out
    }

    fn unescape_ok(src: &str) -> String {
        unescape(src)
            .into_iter()
            .map(|(_, result)| result.expect("valid escape"))
            .collect()
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(unescape_ok("Hello, {}! α"), "Hello, {}! α");
    }

    #[test]
    fn simple_escapes() {
        assert_eq!(unescape_ok(r#"a\nb\tc\r\0\\\"\'"#), "a\nb\tc\r\0\\\"'");
    }

    #[test]
    fn unicode_escapes() {
        assert_eq!(unescape_ok(r"\u{41}\u{1F600}"), "A😀");
    }

    #[test]
    fn ranges_cover_whole_escapes() {
        let ranges: Vec<_> = unescape(r"a\nα").into_iter().map(|(r, _)| r).collect();
        assert_eq!(ranges, [0..1, 1..3, 3..5]);
    }

    #[test]
    fn hex_escapes() {
        assert_eq!(unescape_ok(r"\x41\x7f"), "A\x7f");
    }

    #[test]
    fn byte_strings() {
        let mut bytes = Vec::new();
        unescape_bytes(r"a\xff\n", |_, result| bytes.push(result.unwrap()));
        assert_eq!(bytes, [b'a', 0xff, b'\n']);
        let error = |src| {
            let mut first = None;
            unescape_bytes(src, |_, result| {
                first = first.or(result.err());
            });
            first
        };
        assert_eq!(error("é"), Some(EscapeError::NonAsciiInByteString('é')));
        assert_eq!(
            error(r"\u{41}"),
            Some(EscapeError::UnicodeEscapeInByteString)
        );
    }

    #[test]
    fn errors() {
        let error = |src| unescape(src).into_iter().find_map(|(_, r)| r.err());
        assert_eq!(error(r"\q"), Some(EscapeError::UnknownEscape('q')));
        assert_eq!(error("\\"), Some(EscapeError::LoneBackslash));
        assert_eq!(error(r"\u41"), Some(EscapeError::NoBraceInUnicodeEscape));
        assert_eq!(error(r"\u{41"), Some(EscapeError::UnclosedUnicodeEscape));
        assert_eq!(error(r"\u{}"), Some(EscapeError::EmptyUnicodeEscape));
        assert_eq!(
            error(r"\u{4g}"),
            Some(EscapeError::InvalidCharInUnicodeEscape('g'))
        );
        assert_eq!(
            error(r"\u{1000000}"),
            Some(EscapeError::OverlongUnicodeEscape)
        );
        assert_eq!(error(r"\u{D800}"), Some(EscapeError::InvalidUnicodeEscape));
        assert_eq!(error(r"\x8"), Some(EscapeError::TooShortHexEscape));
        assert_eq!(
            error(r"\xg0"),
            Some(EscapeError::InvalidCharInHexEscape('g'))
        );
        assert_eq!(error(r"\x80"), Some(EscapeError::OutOfRangeHexEscape));
    }
}
