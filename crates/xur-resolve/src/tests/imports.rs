use super::{resolve_files, resolve_src};

const SHAPES: &str = "pub enum Shape { Circle(f64) Square(f64) }\nfn helper() {}\npub fn area() {}";

#[test]
fn std_imports() {
    let resolved = resolve_src("use std;\nuse std::io;\nuse std::io::println;");
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("std"), "module std");
    assert_eq!(resolved.res_of("std::io"), "module std::io");
    assert_eq!(
        resolved.res_of("std::io::println"),
        "builtin function std::io::println"
    );
}

#[test]
fn imports_through_imported_modules() {
    let resolved = resolve_files(&[
        (
            "main.xur",
            "const shapes = std::import(\"shapes.xur\");\nuse shapes::Shape::Circle;\nuse shapes::area;",
        ),
        ("shapes.xur", SHAPES),
    ]);
    resolved.assert_no_errors();
    assert_eq!(
        resolved.res_of("shapes::Shape::Circle"),
        "variant shapes::Shape::Circle"
    );
    assert_eq!(resolved.res_of("shapes::area"), "function shapes::area");
}

#[test]
fn imports_may_depend_on_later_imports() {
    let resolved = resolve_src("use io::println;\nuse std::io;");
    resolved.assert_no_errors();
    assert_eq!(
        resolved.res_of("io::println"),
        "builtin function std::io::println"
    );
}

#[test]
fn prelude_variants_are_reexported() {
    let resolved = resolve_src("use Option::Some;");
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("Option::Some"), "variant Option::Some");
}

#[test]
fn unresolved_imports() {
    assert_eq!(
        resolve_src("use std::nope;").errors,
        ["unresolved import `std::nope`"]
    );
    assert_eq!(
        resolve_src("use nope::x;").errors,
        ["failed to resolve: use of undeclared module `nope`"]
    );
    assert_eq!(
        resolve_src("use nope;").errors,
        ["unresolved import `nope`"]
    );
}

#[test]
fn import_through_a_non_module() {
    assert_eq!(
        resolve_src("fn f() {}\nuse f::x;").errors,
        ["failed to resolve: `f` is a function, not a module"]
    );
}

#[test]
fn private_items_cannot_be_imported() {
    let resolved = resolve_files(&[
        (
            "main.xur",
            "const shapes = std::import(\"shapes.xur\");\nuse shapes::helper;",
        ),
        ("shapes.xur", SHAPES),
    ]);
    assert_eq!(resolved.errors, ["function `helper` is private"]);
}

#[test]
fn failed_imports_do_not_cascade() {
    // The second import finds the `b` declared by the failed first one.
    assert_eq!(
        resolve_src("use a::b;\nuse b::a;").errors,
        ["failed to resolve: use of undeclared module `a`"]
    );
}
