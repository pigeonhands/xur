use super::resolve_src;

#[test]
fn prelude_and_examples_declare_without_errors() {
    resolve_src("").assert_no_errors();
    resolve_src(include_str!("../../../../example/lang/math.xur")).assert_no_errors();
}

#[test]
fn duplicate_items() {
    assert_eq!(
        resolve_src("fn a() {}\nfn a() {}").errors,
        ["the name `a` is defined multiple times"]
    );
    assert_eq!(
        resolve_src("struct S;\nenum S { A }").errors,
        ["the name `S` is defined multiple times"]
    );
    assert_eq!(
        resolve_src("enum E { A, A }").errors,
        ["the name `A` is defined multiple times"]
    );
}

#[test]
fn types_and_values_have_separate_namespaces() {
    resolve_src("struct S { x: u32 }\nfn S() {}").assert_no_errors();
    assert_eq!(
        resolve_src("struct S(u32);\nfn S() {}").errors,
        ["the name `S` is defined multiple times"],
        "a tuple struct also names its constructor"
    );
}

#[test]
fn import_conflicting_with_an_item() {
    assert_eq!(
        resolve_src("use std::io;\nfn io() {}").errors,
        ["the name `io` is defined multiple times"]
    );
}
