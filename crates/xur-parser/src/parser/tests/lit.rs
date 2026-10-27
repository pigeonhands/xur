use xur_ast::{Lit, LitKind};
use xur_span::{Span, Symbol};

use super::parse_errors;
use super::show::Show;
use crate::parser::Parser;

/// Parses a single literal, returning it and the messages of any errors.
fn lit(src: &str) -> (Lit, Vec<String>) {
    let mut parser = Parser::new(src);
    let lit = parser.parse_lit().expect("a literal");
    parser.expect_end().expect("a single literal");
    let errors = parser.finish().into_iter().map(|d| d.message).collect();
    (lit, errors)
}

fn lit_ok(src: &str) -> String {
    let (lit, errors) = lit(src);
    assert_eq!(errors, Vec::<String>::new(), "errors in {src:?}");
    lit.show()
}

#[test]
fn integers() {
    assert_eq!(lit_ok("42"), "42");
    assert_eq!(lit_ok("1_000_000"), "1000000");
    assert_eq!(lit_ok("18446744073709551615"), "18446744073709551615");
}

#[test]
fn integer_bases() {
    assert_eq!(lit_ok("0x1F"), "31");
    assert_eq!(lit_ok("0xdead_BEEF"), "3735928559");
    assert_eq!(lit_ok("0o17"), "15");
    assert_eq!(lit_ok("0b1010"), "10");
    assert_eq!(lit("0xg").1, ["invalid digit in integer literal"]);
    assert_eq!(
        lit("0x").1,
        ["missing digits after the integer base prefix"]
    );
}

#[test]
fn byte_strings() {
    assert_eq!(lit_ok(r#"b"MZ\x90\x00""#), r#"b"MZ\x90\x00""#);
}

#[test]
fn integer_overflow_is_reported_and_recovered() {
    let (lit, errors) = lit("18446744073709551616");
    assert_eq!(lit.kind, LitKind::Err);
    assert_eq!(errors, ["integer literal is too large"]);
}

#[test]
fn floats_keep_their_digits() {
    let (lit, _) = lit("1_000.5_0");
    assert_eq!(lit.kind, LitKind::Float(Symbol::intern("1000.50")));
}

#[test]
fn strings_are_unescaped() {
    assert_eq!(lit_ok(r#""a\tb\u{41}""#), r#""a\tbA""#);
}

#[test]
fn bools() {
    assert_eq!(lit_ok("true"), "true");
    assert_eq!(lit_ok("false"), "false");
}

#[test]
fn every_bad_escape_is_reported_at_its_position() {
    let mut parser = Parser::new(r#""a\q \u{}""#);
    let lit = parser.parse_lit().unwrap();
    assert_eq!(lit.kind, LitKind::Err);
    let diags = parser.finish();
    let found: Vec<_> = diags.iter().map(|d| (d.message.as_str(), d.span)).collect();
    assert_eq!(
        found,
        [
            ("unknown character escape: `q`", Span::new(2, 4)),
            ("empty unicode escape", Span::new(5, 9)),
        ]
    );
}

#[test]
fn non_literal() {
    assert_eq!(
        parse_errors("x", Parser::parse_lit),
        ["expected literal, found `x`"]
    );
}
