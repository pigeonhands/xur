use xur_ast::PatKind;

use super::show::Show;
use super::{parse_errors, parse_ok};
use crate::parser::Parser;

fn pat(src: &str) -> String {
    parse_ok(src, Parser::parse_pat).show()
}

#[test]
fn simple_patterns() {
    assert_eq!(pat("_"), "_");
    assert_eq!(pat("x"), "x");
    assert_eq!(pat("0"), "0");
    assert_eq!(pat(r#""text""#), r#""text""#);
}

#[test]
fn lone_identifiers_are_bindings_longer_paths_are_not() {
    assert!(matches!(
        parse_ok("x", Parser::parse_pat).kind,
        PatKind::Ident(_)
    ));
    assert!(matches!(
        parse_ok("MathError::DivideByZero", Parser::parse_pat).kind,
        PatKind::Path(_)
    ));
}

#[test]
fn tuple_struct_patterns() {
    assert_eq!(pat("Ok(v)"), "Ok(v)");
    assert_eq!(pat("Shape::Rect(w, _)"), "Shape::Rect(w, _)");
    assert_eq!(
        pat("Err(math::MathError::DivideByZero)"),
        "Err(math::MathError::DivideByZero)"
    );
}

#[test]
fn struct_patterns() {
    assert_eq!(pat("Point { x, y: 0 }"), "Point { x, y: 0 }");
    assert_eq!(
        pat("Event::Hit { damage, .. }"),
        "Event::Hit { damage, .. }"
    );
    assert_eq!(pat("Empty {}"), "Empty {  }");
    assert_eq!(
        parse_errors("P { .., x }", Parser::parse_pat),
        ["`..` must be the last field of a pattern"]
    );
}

#[test]
fn tuple_and_paren_patterns() {
    assert_eq!(pat("()"), "()");
    assert_eq!(pat("(a)"), "(a)");
    assert_eq!(pat("(a,)"), "(a,)");
    assert_eq!(pat("(a, (b, _))"), "(a, (b, _))");
}

#[test]
fn missing_pattern() {
    assert_eq!(
        parse_errors("=>", Parser::parse_pat),
        ["expected pattern, found `=>`"]
    );
}
