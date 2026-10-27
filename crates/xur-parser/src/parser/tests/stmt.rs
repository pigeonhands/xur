use xur_span::Span;

use super::show::Show;
use super::{parse_errors, parse_ok};
use crate::parser::Parser;

fn block(src: &str) -> String {
    parse_ok(src, Parser::parse_block).show()
}

/// Parses a block, returning what was recovered and the error messages.
fn block_with_errors(src: &str) -> (String, Vec<String>) {
    let mut parser = Parser::new(src);
    let block = parser.parse_block().expect("block recovers");
    let errors = parser.finish().into_iter().map(|d| d.message).collect();
    (block.show(), errors)
}

#[test]
fn let_statements() {
    assert_eq!(block("{ let x = 1; }"), "{ let x = 1; }");
    assert_eq!(block("{ let x: u32 = 1; }"), "{ let x: u32 = 1; }");
    assert_eq!(block("{ let x; }"), "{ let x; }");
    assert_eq!(block("{ let (a, _) = t; }"), "{ let (a, _) = t; }");
}

#[test]
fn trailing_expression_is_the_value() {
    assert_eq!(block("{ a; b }"), "{ a; b }");
    assert_eq!(block("{ ; }"), "{ ; }");
    assert_eq!(block("{}"), "{}");
}

#[test]
fn block_like_statements_need_no_semicolon() {
    assert_eq!(
        block("{ if c { a } for i in x { } b }"),
        "{ if c { a } for i in x {} b }"
    );
}

#[test]
fn block_like_statement_ends_before_operator() {
    // As in Rust: an `if` statement followed by the expression `-1`.
    assert_eq!(block("{ if c { a } -1 }"), "{ if c { a } (-1) }");
    assert_eq!(block("{ { a } (1, 2) }"), "{ { a } (1, 2) }");
    assert_eq!(block("{ { a } [1] }"), "{ { a } [1] }");
    assert_eq!(block("{ let x = { a } - 1; }"), "{ let x = ({ a } - 1); }");
}

#[test]
fn method_call_on_block_like_statement() {
    assert_eq!(
        block("{ match x { _ => a }.unwrap() }"),
        "{ match x { _ => a }.unwrap() }"
    );
}

#[test]
fn block_span() {
    let block = parse_ok("{ a; }", Parser::parse_block);
    assert_eq!(block.span, Span::new(0, 6));
}

#[test]
fn missing_semicolon_is_recovered() {
    let (block, errors) = block_with_errors("{ let x = 1 let y = 2; y }");
    assert_eq!(
        errors,
        ["expected one of `(`, `.`, `;`, `?`, `[`, `as`, `is`, or an operator, found keyword `let`"]
    );
    assert_eq!(block, "{ <err> y }");
}

#[test]
fn missing_semicolon_at_end_of_line_is_assumed() {
    let (block, errors) = block_with_errors("{\n    let x = 1\n    x = 2\n    y\n}");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(block, "{ let x = 1; (x = 2); y }");
}

#[test]
fn errors_after_a_missing_semicolon_are_still_found() {
    let (_, errors) = block_with_errors("{\n    let x = 1\n    let y = a < b < c;\n}");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(errors[1], "comparison operators cannot be chained");
}

#[test]
fn every_bad_statement_is_reported() {
    let (block, errors) = block_with_errors("{ a b; f(1 2); ok(); }");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(block, "{ <err> <err> ok(); }");
}

#[test]
fn missing_semicolon_points_to_end_of_line() {
    let mut parser = Parser::new("{\n    let x = 1\n    let y = 2;\n}");
    parser.parse_block().unwrap();
    let diag = &parser.finish()[0];
    assert_eq!(diag.span, Span::new(15, 15)); // just after `1`
    assert_eq!(diag.labels[1].message, "unexpected token");
}

#[test]
fn unclosed_block() {
    assert_eq!(
        parse_errors("{ let x = 1;", Parser::parse_block),
        ["this file contains an unclosed delimiter"]
    );
}
