use xur_span::Span;

use super::show::Show;
use super::{parse_errors, parse_ok};
use crate::parser::Parser;

fn expr(src: &str) -> String {
    parse_ok(src, Parser::parse_expr).show()
}

fn expr_errors(src: &str) -> Vec<String> {
    parse_errors(src, Parser::parse_expr)
}

#[test]
fn primaries() {
    assert_eq!(expr("42"), "42");
    assert_eq!(expr("x"), "x");
    assert_eq!(expr("std::io"), "std::io");
    assert_eq!(expr(r#""hi""#), r#""hi""#);
}

#[test]
fn arithmetic_precedence() {
    assert_eq!(expr("1 + 2 * 3"), "(1 + (2 * 3))");
    assert_eq!(expr("1 * 2 + 3"), "((1 * 2) + 3)");
    assert_eq!(expr("a % b - c / d"), "((a % b) - (c / d))");
}

#[test]
fn left_associativity() {
    assert_eq!(expr("a - b - c"), "((a - b) - c)");
    assert_eq!(expr("a / b * c"), "((a / b) * c)");
}

#[test]
fn logic_and_comparison() {
    assert_eq!(expr("a || b && c"), "(a || (b && c))");
    assert_eq!(expr("a == b && c < d"), "((a == b) && (c < d))");
    assert_eq!(expr("i % 2 == 0"), "((i % 2) == 0)");
}

#[test]
fn assignment_is_right_associative_and_loosest() {
    assert_eq!(expr("a = b = c"), "(a = (b = c))");
    assert_eq!(expr("a = b || c"), "(a = (b || c))");
    assert_eq!(expr("x = 1..2"), "(x = (1..2))");
}

#[test]
fn prefix_operators() {
    assert_eq!(expr("-a * b"), "((-a) * b)");
    assert_eq!(expr("!a.b()"), "(!a.b())");
    assert_eq!(expr("--x"), "(-(-x))");
}

#[test]
fn postfix_operators() {
    assert_eq!(expr("math.div(total, count)?"), "math.div(total, count)?");
    assert_eq!(expr("a.b.c(1)(2)"), "a.b.c(1)(2)");
    assert_eq!(expr("f()?.g"), "f()?.g");
    assert_eq!(
        expr("std::any::type_name::<T>()"),
        "std::any::type_name<T>()"
    );
    assert_eq!(expr("it.collect::<Vec<T>>()"), "it.collect::<Vec<T>>()");
}

#[test]
fn ranges() {
    assert_eq!(expr("0..10"), "(0..10)");
    assert_eq!(expr("1..=5"), "(1..=5)");
    assert_eq!(expr("0..n + 1"), "(0..(n + 1))");
    assert_eq!(expr("a.."), "(a..)");
    assert_eq!(expr("..b"), "(..b)");
    assert_eq!(expr(".."), "(..)");
}

#[test]
fn paren_and_tuple() {
    assert_eq!(expr("()"), "()");
    assert_eq!(expr("(1)"), "(1)");
    assert_eq!(expr("(1,)"), "(1,)");
    assert_eq!(expr("(1, 2)"), "(1, 2)");
    assert_eq!(expr("(1 + 2) * 3"), "(((1 + 2)) * 3)");
}

#[test]
fn closures() {
    assert_eq!(expr("|x: u32| x * 2"), "(|x: u32| (x * 2))");
    assert_eq!(expr("|| 1"), "(|| 1)");
    assert_eq!(expr("|a, b| a + b"), "(|a, b| (a + b))");
    assert_eq!(
        expr("|x| { let y = double(x); y + offset }"),
        "(|x| { let y = double(x); (y + offset) })"
    );
    assert_eq!(expr("|x| -> u32 { x }"), "(|x| -> u32 { x })");
}

#[test]
fn closure_body_extends_right() {
    assert_eq!(expr("|x| x = 1"), "(|x| (x = 1))");
}

#[test]
fn if_is_an_expression() {
    assert_eq!(
        expr(r#"if i % 2 == 0 { "even" } else { "odd" }"#),
        r#"if ((i % 2) == 0) { "even" } else { "odd" }"#
    );
    assert_eq!(
        expr("if a { 1 } else if b { 2 } else { 3 }"),
        "if a { 1 } else if b { 2 } else { 3 }"
    );
    assert_eq!(
        expr("1 + if a { 2 } else { 3 } * 4"),
        "(1 + (if a { 2 } else { 3 } * 4))"
    );
}

#[test]
fn match_arms() {
    assert_eq!(
        expr("match average(10, 0) { Ok(avg) => avg, Err(_) => 0 }"),
        "match average(10, 0) { Ok(avg) => avg, Err(_) => 0 }"
    );
}

#[test]
fn block_like_arms_need_no_comma() {
    assert_eq!(
        expr("match s { A => { 1 } B => if c { 2 } else { 3 } _ => 4 }"),
        "match s { A => { 1 }, B => if c { 2 } else { 3 }, _ => 4 }"
    );
}

#[test]
fn loops() {
    assert_eq!(
        expr("for i in 0..10 { f(i); }"),
        "for i in (0..10) { f(i); }"
    );
    assert_eq!(expr("for i in 0.. { }"), "for i in (0..) {}");
    assert_eq!(
        expr("while x < 3 { x = x + 1; }"),
        "while (x < 3) { (x = (x + 1)); }"
    );
    assert_eq!(expr("loop { break; }"), "loop { break; }");
}

#[test]
fn jumps() {
    assert_eq!(expr("return"), "return");
    assert_eq!(expr("return a + 1"), "(return (a + 1))");
    assert_eq!(expr("loop { break 1 }"), "loop { (break 1) }");
    assert_eq!(expr("loop { continue }"), "loop { continue }");
}

#[test]
fn spans_cover_whole_expressions() {
    let e = parse_ok("a + b * c", Parser::parse_expr);
    assert_eq!(e.span, Span::new(0, 9));
    let e = parse_ok("math.add(1, i)", Parser::parse_expr);
    assert_eq!(e.span, Span::new(0, 14));
}

#[test]
fn chained_comparisons_are_rejected() {
    assert_eq!(
        expr_errors("a < b < c"),
        ["comparison operators cannot be chained"]
    );
    assert_eq!(
        expr_errors("a..b..c"),
        ["range operators cannot be chained"]
    );
    assert_eq!(expr_errors("(a < b) < c"), Vec::<String>::new());
}

#[test]
fn inclusive_range_needs_an_end() {
    assert_eq!(expr_errors("a..="), ["inclusive range with no end"]);
}

#[test]
fn missing_condition() {
    assert_eq!(
        expr_errors("if { 1 }"),
        ["missing condition for `if` expression"]
    );
}

#[test]
fn missing_operand() {
    assert_eq!(expr_errors("1 + ;"), ["expected expression, found `;`"]);
}

#[test]
fn unclosed_call() {
    assert_eq!(
        expr_errors("f(1 2)"),
        ["expected one of `(`, `)`, `,`, `.`, `?`, `[`, or an operator, found `2`"]
    );
}

#[test]
fn match_arm_needs_comma() {
    assert_eq!(
        expr_errors("match x { A => 1 B => 2 }"),
        ["expected one of `(`, `,`, `.`, `?`, `[`, `}`, or an operator, found `B`"]
    );
}

#[test]
fn generic_args_on_field() {
    assert_eq!(
        expr_errors("a.b::<T>"),
        ["field expressions cannot have generic arguments"]
    );
}

#[test]
fn lists_and_indexing() {
    assert_eq!(expr("[]"), "[]");
    assert_eq!(expr("[1, 2 + 3,]"), "[1, (2 + 3)]");
    assert_eq!(expr("xs[0]"), "xs[0]");
    assert_eq!(expr("xs[i + 1].add(2)"), "xs[(i + 1)].add(2)");
    assert_eq!(expr("-xs[0]"), "(-xs[0])");
    assert_eq!(expr("f()[0][1]"), "f()[0][1]");
}

#[test]
fn struct_literals() {
    assert_eq!(expr("Point { x: 1, y }"), "Point { x: 1, y }");
    assert_eq!(
        expr("Event::Hit { damage: 2 + 3, }"),
        "Event::Hit { damage: (2 + 3) }"
    );
    assert_eq!(expr("Empty {}"), "Empty {  }");
    assert_eq!(expr("Point { x: 1 }.x"), "Point { x: 1 }.x");
}

#[test]
fn no_struct_literals_before_blocks() {
    // `x {` begins the block of the `if`, not a struct literal.
    assert_eq!(expr("if x { 1 } else { 2 }"), "if x { 1 } else { 2 }");
    assert_eq!(
        expr("match p { P { x, .. } => x }"),
        "match p { P { x, .. } => x }"
    );
    assert_eq!(
        expr("if (P { x: 1 }).x == 1 { 2 }"),
        "if ((P { x: 1 }).x == 1) { 2 }"
    );
}
