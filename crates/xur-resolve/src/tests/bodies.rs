use super::{resolve_files, resolve_src};

#[test]
fn params_and_locals() {
    let resolved = resolve_src("fn f(a: u32) -> u32 {\n    let b = a;\n    b\n}");
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of_nth("a", 1), "local a at main.xur:1:6");
    assert_eq!(resolved.res_of_nth("b", 1), "local b at main.xur:2:9");
    assert_eq!(resolved.res_of("u32"), "builtin type u32");
}

#[test]
fn shadowing() {
    let resolved = resolve_src("fn f() {\n    let x = 1;\n    let x = x;\n    x\n}");
    resolved.assert_no_errors();
    // Occurrences in visiting order: binding, initializer, binding, tail.
    // The initializer sees the first `x`; the tail sees the second.
    assert_eq!(resolved.res_of_nth("x", 1), "local x at main.xur:2:9");
    assert_eq!(resolved.res_of_nth("x", 3), "local x at main.xur:3:9");
}

#[test]
fn locals_are_scoped_to_their_block() {
    assert_eq!(
        resolve_src("fn f() { { let x = 1; } x }").errors,
        ["cannot find value `x` in this scope"]
    );
}

#[test]
fn nested_functions_cannot_see_outer_locals() {
    assert_eq!(
        resolve_src("fn f() { let x = 1; fn g() { x } }").errors,
        ["cannot find value `x` in this scope"]
    );
}

#[test]
fn block_items_are_visible_throughout_the_block() {
    let resolved = resolve_src("fn f() { g(); fn g() { h() } fn h() {} }");
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("g"), "function g");
}

#[test]
fn closures_capture_enclosing_locals() {
    let resolved = resolve_src("fn f() { let y = 1; let add = |x: u32| x + y; add(2) }");
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of_nth("y", 1), "local y at main.xur:1:14");
    assert_eq!(resolved.res_of_nth("x", 1), "local x at main.xur:1:32");
}

#[test]
fn loop_and_arm_bindings() {
    let resolved = resolve_src(
        "fn f(r: Result<u32, u32>) {\n    for i in 0..3 { i; }\n    match r { Ok(v) => v, Err(_) => 0 }\n}",
    );
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of_nth("i", 1), "local i at main.xur:2:9");
    assert_eq!(resolved.res_of("Ok"), "variant Result::Ok");
    assert_eq!(resolved.res_of_nth("v", 1), "local v at main.xur:3:18");
    assert_eq!(resolved.res_of("Result<u32, u32>"), "enum Result");
}

#[test]
fn identifier_patterns_match_unit_variants_or_bind() {
    let resolved = resolve_src("fn f(o: Option<u32>) { match o { None => 0, x => 1 } }");
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("None"), "variant Option::None");
    assert_eq!(resolved.res_of_nth("x", 0), "local x at main.xur:1:45");
}

#[test]
fn generics() {
    let resolved = resolve_src("fn id<T>(x: T) -> T { x }\nstruct Wrapper<T> { inner: T }");
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("T"), "type parameter id::T");
    assert_eq!(resolved.res_of_nth("T", 2), "type parameter Wrapper::T");
}

#[test]
fn turbofish_types_are_resolved() {
    let resolved = resolve_src("struct S;\nfn f() { std::any::type_name::<S>(); }");
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("S"), "struct S");
}

#[test]
fn unknown_names() {
    assert_eq!(
        resolve_src("fn f(x: Q) { y }").errors,
        [
            "cannot find type `Q` in this scope",
            "cannot find value `y` in this scope"
        ]
    );
    assert_eq!(
        resolve_src("fn f() { std::io::nope() }").errors,
        ["cannot find value `nope` in module `std::io`"]
    );
}

#[test]
fn locals_are_not_modules() {
    assert_eq!(
        resolve_src("fn f(x: u32) { x::y }").errors,
        ["failed to resolve: `x` is a local variable, not a module"]
    );
}

#[test]
fn modules_are_values() {
    let resolved = resolve_src("fn main() {\n    let io = std::io;\n    io::println(\"hi\");\n}");
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("std::io"), "module std::io");
    assert_eq!(
        resolved.res_of("io::println"),
        "builtin function std::io::println"
    );
}

#[test]
fn paths_into_imported_modules() {
    let resolved = resolve_files(&[
        (
            "main.xur",
            "const shapes = std::import(\"shapes.xur\");\nfn f() { shapes::area(); shapes::helper(); shapes.area(); }",
        ),
        ("shapes.xur", "pub fn area() {}\nfn helper() {}"),
    ]);
    assert_eq!(resolved.errors, ["function `helper` is private"]);
    assert_eq!(resolved.res_of("shapes::area"), "function shapes::area");
    // `shapes.area()` is a method call on a module value: only `shapes` is a
    // name; `area` is a member, resolved later.
    assert_eq!(resolved.res_of("shapes"), "module shapes");
}

#[test]
fn duplicate_bindings_in_a_pattern() {
    assert_eq!(
        resolve_src("fn f(t: (u32, u32)) { let (a, a) = t; }").errors,
        ["identifier `a` is bound more than once in the same pattern"]
    );
}

#[test]
fn std_import_only_initialises_top_level_consts() {
    assert_eq!(
        resolve_src("fn f() { let m = std::import(\"m.xur\"); }").errors,
        ["`std::import` can only initialise a top-level `const`"]
    );
}

#[test]
fn use_only_at_top_level() {
    assert_eq!(
        resolve_src("fn f() { use std::io; }").errors,
        ["`use` is only allowed at the top level of a module"]
    );
}

#[test]
fn examples_resolve_cleanly() {
    resolve_files(&[
        ("main.xur", include_str!("../../../../example/main.xur")),
        ("math.xur", include_str!("../../../../example/math.xur")),
    ])
    .assert_no_errors();
}
