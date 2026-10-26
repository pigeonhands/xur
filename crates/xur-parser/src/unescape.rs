//! Decoding of escape sequences in string literals, after
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
        }
    }
}

/// Decodes the contents of a string literal (without the quotes), calling
/// `callback` with the byte range and result of each character or escape.
pub fn unescape_str(src: &str, mut callback: impl FnMut(Range<usize>, Result<char, EscapeError>)) {
    let offset = |rest: &Chars<'_>| src.len() - rest.as_str().len();
    let mut chars = src.chars();
    loop {
        let start = offset(&chars);
        let result = match chars.next() {
            None => return,
            Some('\\') => scan_escape(&mut chars),
            Some(c) => Ok(c),
        };
        callback(start..offset(&chars), result);
    }
}

/// Decodes an escape whose `\` was already consumed.
fn scan_escape(chars: &mut Chars<'_>) -> Result<char, EscapeError> {
    let c = match chars.next().ok_or(EscapeError::LoneBackslash)? {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        '0' => '\0',
        '\\' => '\\',
        '"' => '"',
        '\'' => '\'',
        'u' => return scan_unicode(chars),
        other => return Err(EscapeError::UnknownEscape(other)),
    };
    Ok(c)
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
    }
}
