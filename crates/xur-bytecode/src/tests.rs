use std::path::Path;

use xur_resolve::{DefKind, Namespace, Program, Res};
use xur_span::{MemoryFileLoader, Symbol};

use crate::{ItemIndex, display_function, lower};

/// The bytecode listing of function `name` in a program made of `src`.
fn listing(src: &str, name: &str) -> String {
    let loader = MemoryFileLoader::new([("main.xur", src)]);
    let natives = Box::new(xur_std::registry());
    let (program, diags) = Program::load(Path::new("main.xur"), Box::new(loader), natives).unwrap();
    assert!(diags.is_empty(), "{diags:#?}");
    let resolutions = program.resolutions();
    let root = resolutions.module_defs[program.krate.root.0];
    let Some(Res::Def(DefKind::Fn, def)) =
        resolutions.lookup(root, Namespace::Value, Symbol::intern(name))
    else {
        panic!("no function `{name}`");
    };
    let items = ItemIndex::new(&program);
    let function = lower(&program, &items, def).expect("a function");
    display_function(&function, resolutions)
}

#[test]
fn arithmetic() {
    insta_like(
        &listing("fn add(a: i64, b: i64) -> i64 { a + b * 2 }", "add"),
        "fn add (captures: 0, params: 2, registers: 5)
  bb0:
    r2 = 2
    r3 = r1 * r2
    r4 = r0 + r3
    return r4
",
    );
}

/// Compares a listing, printing it in full on mismatch.
fn insta_like(actual: &str, expected: &str) {
    assert!(
        actual == expected,
        "expected:\n{expected}\nactual:\n{actual}"
    );
}

#[test]
fn if_is_an_expression() {
    insta_like(
        &listing("fn f(c: bool) -> i64 { if c { 1 } else { 2 } }", "f"),
        "fn f (captures: 0, params: 1, registers: 4)
  bb0:
    branch r0 ? bb1 : bb2
  bb1:
    r2 = 1
    r1 = r2
    goto bb3
  bb2:
    r3 = 2
    r1 = r3
    goto bb3
  bb3:
    return r1
",
    );
}

#[test]
fn match_tests_each_arm() {
    insta_like(
        &listing(
            "fn f(o: Option<i64>) -> i64 { match o { Some(x) => x, None => 0 } }",
            "f",
        ),
        "fn f (captures: 0, params: 1, registers: 7)
  bb0:
    r2 = r0 is Option::Some
    branch r2 ? bb3 : bb2
  bb1:
    return r1
  bb2:
    r5 = r0 is Option::None
    branch r5 ? bb5 : bb4
  bb3:
    r3 = r0.0
    r4 = r3
    r1 = r4
    goto bb1
  bb4:
    no match
  bb5:
    r6 = 0
    r1 = r6
    goto bb1
",
    );
}

#[test]
fn try_returns_errors_early() {
    insta_like(
        &listing(
            "fn f(r: Result<i64, i64>) -> Result<i64, i64> { let v = r?; Ok(v + 1) }",
            "f",
        ),
        "fn f (captures: 0, params: 1, registers: 8)
  bb0:
    r2 = r0 is Result::Ok
    branch r2 ? bb1 : bb2
  bb1:
    r1 = r0.0
    goto bb4
  bb2:
    r3 = r0 is Option::Some
    branch r3 ? bb1 : bb3
  bb3:
    return r0
  bb4:
    r4 = r1
    r5 = 1
    r6 = r4 + r5
    r7 = Result::Ok(r6)
    return r7
",
    );
}

#[test]
fn for_loops_iterate() {
    insta_like(
        &listing(
            "fn f(xs: [i64]) -> i64 { let total = 0; for x in xs { total = total + x; } total }",
            "f",
        ),
        "fn f (captures: 0, params: 1, registers: 10)
  bb0:
    r1 = 0
    r2 = r1
    r3 = iter r0
    goto bb1
  bb1:
    r4 = next r3 ? bb2 : bb3
  bb2:
    r5 = ()
    r6 = r2 + r4
    r2 = r6
    r7 = ()
    r8 = ()
    goto bb1
  bb3:
    r9 = ()
    return r2
",
    );
}

#[test]
fn closures_capture_values() {
    insta_like(
        &listing("fn f(k: i64) { let add = |x| x + k; add(1); }", "f"),
        "fn f (captures: 0, params: 1, registers: 6)
  bb0:
    r1 = closure#0(r0)
    r2 = r1
    r3 = 1
    r4 = call r2(r3)
    r5 = ()
    return r5

fn f::closure#0 (captures: 1, params: 1, registers: 3)
  bb0:
    r2 = r1 + r0
    return r2
",
    );
}

#[test]
fn and_short_circuits() {
    insta_like(
        &listing("fn f(a: bool, b: bool) -> bool { a && b }", "f"),
        "fn f (captures: 0, params: 2, registers: 3)
  bb0:
    r2 = r0
    branch r0 ? bb1 : bb2
  bb1:
    r2 = r1
    goto bb2
  bb2:
    return r2
",
    );
}
