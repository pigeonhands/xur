use xur_span::Span;

use super::show::Show;
use super::{parse_errors, parse_ok};
use crate::parser::{Parser, PathStyle};

fn ty(src: &str) -> String {
    parse_ok(src, Parser::parse_ty).show()
}

#[test]
fn path_types() {
    assert_eq!(ty("u32"), "u32");
    assert_eq!(ty("math::MathError"), "math::MathError");
}

#[test]
fn generic_types() {
    assert_eq!(ty("Result<u32, MathError>"), "Result<u32, MathError>");
    assert_eq!(ty("Vec<Result<T, E>,>"), "Vec<Result<T, E>>");
    assert_eq!(ty("std::Rc<T>::Inner"), "std::Rc<T>::Inner");
}

#[test]
fn tuple_and_paren_types() {
    assert_eq!(ty("()"), "()");
    assert_eq!(ty("(u32)"), "(u32)");
    assert_eq!(ty("(u32,)"), "(u32,)");
    assert_eq!(ty("(u32, (A, B))"), "(u32, (A, B))");
}

#[test]
fn infer_type() {
    assert_eq!(ty("_"), "_");
}

#[test]
fn type_span_covers_generic_args() {
    let ty = parse_ok("Result<u32, E>", Parser::parse_ty);
    assert_eq!(ty.span, Span::new(0, 14));
}

#[test]
fn expression_paths_need_turbofish() {
    let path = |src| parse_ok(src, |p| p.parse_path(PathStyle::Expr)).show();
    assert_eq!(path("std::any::type_name::<T>"), "std::any::type_name<T>");
    assert_eq!(path("Vec::<T>::new"), "Vec<T>::new");
}

#[test]
fn module_paths_take_no_generic_args() {
    let errors = parse_errors("std::<T>", |p| {
        p.parse_path(PathStyle::Mod)?;
        p.expect_end()
    });
    assert_eq!(errors, ["expected identifier, found `<`"]);
}

#[test]
fn missing_type() {
    assert_eq!(
        parse_errors(";", Parser::parse_ty),
        ["expected type, found `;`"]
    );
}

#[test]
fn unclosed_generic_args() {
    assert_eq!(
        parse_errors("Result<u32 E>", Parser::parse_ty),
        ["expected one of `,`, `::`, `<`, or `>`, found `E`"]
    );
}
