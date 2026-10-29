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
    // Occurrences in source order: binding, binding, initializer, tail.
    // The initializer sees the first `x`; the tail sees the second.
    assert_eq!(resolved.res_of_nth("x", 2), "local x at main.xur:2:9");
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
        "native function std::io::println"
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
fn imports_are_ordinary_calls() {
    let resolved = resolve_files(&[
        (
            "main.xur",
            "fn load(path: str) {\n    let mods = [std::import(path), std::import(\"shapes.xur\")];\n    mods[0].area();\n}",
        ),
        ("shapes.xur", "pub fn area() {}"),
    ]);
    resolved.assert_no_errors();
    assert_eq!(
        resolved.res_of("std::import"),
        "native function std::import"
    );
    assert_eq!(resolved.res_of_nth("mods", 1), "local mods at main.xur:2:9");
}

#[test]
fn a_local_initialised_with_a_literal_import_prefixes_paths() {
    let resolved = resolve_files(&[
        (
            "main.xur",
            "fn f() {\n    let shapes = std::import(\"shapes.xur\");\n    shapes::area();\n}",
        ),
        ("shapes.xur", "pub fn area() {}"),
    ]);
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("shapes::area"), "function shapes::area");
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
        ("main.xur", include_str!("../../../../example/lang/main.xur")),
        ("math.xur", include_str!("../../../../example/lang/math.xur")),
    ])
    .assert_no_errors();
}

#[test]
fn native_types_and_their_functions() {
    let resolved = resolve_src(
        "use geo::Point;\nfn f(p: Point) -> f64 { let q = geo::Point::new(1.0, 2.0); Point::norm(q) }",
    );
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("Point"), "native type geo::Point");
    assert_eq!(
        resolved.res_of("geo::Point::new"),
        "native function geo::Point::new"
    );
    assert_eq!(
        resolved.res_of("Point::norm"),
        "native function geo::Point::norm"
    );
}

#[test]
fn native_enum_variants_are_values_and_types() {
    let resolved = resolve_src(
        "use geo::Machine;\n\
         fn f(m: Machine) {\n\
             Machine::Amd64; Machine::Other(1); Machine::Named { x: 1 };\n\
             match m { Machine::Amd64 => 1, Machine::Other(n) => n, Machine::Named { x } => x, }\n\
         }",
    );
    resolved.assert_no_errors();
    assert_eq!(
        resolved.res_of("Machine::Amd64"),
        "variant geo::Machine::Amd64"
    );
    assert_eq!(
        resolved.res_of("Machine::Named"),
        "variant geo::Machine::Named"
    );
    assert_eq!(
        resolved.res_of("Machine::Other"),
        "variant geo::Machine::Other"
    );
}

#[test]
fn grouped_imports_import_each_path() {
    let resolved = resolve_src(
        "use std::{io, any::type_name};\nfn f() { io::println(\"\"); type_name::<i32>(); }",
    );
    resolved.assert_no_errors();
    assert_eq!(
        resolved.res_of("io::println"),
        "native function std::io::println"
    );
    assert_eq!(
        resolved.res_of("type_name::<i32>"),
        "native function std::any::type_name"
    );
}

#[test]
fn shadowing_builtin_type_is_an_error() {
    assert_eq!(
        resolve_src("fn f() { let bytes = 1; }").errors,
        ["`bytes` is a builtin type and cannot be used as a variable name"]
    );
    assert_eq!(
        resolve_src("fn f() { let str = 1; }").errors,
        ["`str` is a builtin type and cannot be used as a variable name"]
    );
}

#[test]
fn non_builtin_names_are_fine() {
    resolve_src("fn f() { let data = 1; }").assert_no_errors();
}
