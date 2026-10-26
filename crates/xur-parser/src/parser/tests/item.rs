use xur_ast::ItemKind;

use super::show::Show;
use super::{parse_errors, parse_ok};
use crate::parser::{Parser, parse_module};

fn item(src: &str) -> String {
    parse_ok(src, Parser::parse_item).show()
}

#[test]
fn use_and_const() {
    assert_eq!(item("use std;"), "use std;");
    assert_eq!(item("use std::io;"), "use std::io;");
    assert_eq!(
        item(r#"const math = std::import("./math.xur");"#),
        r#"const math = std::import("./math.xur");"#
    );
    assert_eq!(item("const N: u32 = 4;"), "const N: u32 = 4;");
}

#[test]
fn functions() {
    assert_eq!(
        item("pub fn add(a: u32, b: u32) -> u32 { a + b }"),
        "pub fn add(a: u32, b: u32) -> u32 { (a + b) }"
    );
    assert_eq!(item("fn main() {}"), "fn main() {}");
    assert_eq!(
        item("fn print_name<T>(s: T,) {}"),
        "fn print_name<T>(s: T) {}"
    );
}

#[test]
fn struct_fields_may_be_separated_by_newlines() {
    assert_eq!(
        item("struct CPacked {\n    a: usize\n    b: u32\n}"),
        "struct CPacked { a: usize, b: u32 }"
    );
    assert_eq!(
        item("struct S<T> { pub a: T, b: u32, }"),
        "struct S<T> { pub a: T, b: u32 }"
    );
}

#[test]
fn tuple_and_unit_structs() {
    assert_eq!(item("struct Meters(f64);"), "struct Meters(f64);");
    assert_eq!(item("struct Marker;"), "struct Marker;");
}

#[test]
fn attributes() {
    assert_eq!(item("#[repr(C)] struct A;"), "#[repr(C)] struct A;");
    assert_eq!(
        item(
            "#[repr(align(4))]\n#[endian(Little)]\nstruct B {\n    #[field(offset=100)] c: u32\n}"
        ),
        "#[repr(align(4))] #[endian(Little)] struct B { #[field(offset = 100)] c: u32 }"
    );
    assert_eq!(item("#[doc = \"x\"] use std;"), "#[doc = \"x\"] use std;");
}

#[test]
fn enums() {
    assert_eq!(
        item("pub enum MathError {\n    DivideByZero\n    Overflow\n}"),
        "pub enum MathError { DivideByZero, Overflow }"
    );
    assert_eq!(
        item("enum Shape { Circle(f64) Rect(f64, f64) }"),
        "enum Shape { Circle(f64), Rect(f64, f64) }"
    );
    assert_eq!(
        item("enum Result<T, E> { Ok(T), Err(E), }"),
        "enum Result<T, E> { Ok(T), Err(E) }"
    );
    assert_eq!(
        item("enum Event { Click { x: u32 y: u32 } }"),
        "enum Event { Click { x: u32, y: u32 } }"
    );
}

#[test]
fn items_nested_in_blocks() {
    assert_eq!(
        item("fn main() { fn helper() {} helper() }"),
        "fn main() { fn helper() {} helper() }"
    );
}

#[test]
fn expected_item() {
    assert_eq!(
        parse_errors("let x = 1;", Parser::parse_item),
        ["expected item, found keyword `let`"]
    );
}

#[test]
fn broken_items_are_skipped() {
    let (module, diags) = parse_module(
        "fn ok() {}\nfn broken(a u32) { if x { fn inner() {} } }\nstruct AlsoOk;\npub pub fn f() {}",
    );
    let names: Vec<_> = module
        .items
        .iter()
        .map(|item| match &item.kind {
            ItemKind::Fn(f) => f.ident.to_string(),
            ItemKind::Struct(ident, ..) => ident.to_string(),
            _ => "?".to_owned(),
        })
        .collect();
    assert_eq!(names, ["ok", "AlsoOk", "f"]);
    assert_eq!(diags.len(), 2, "{diags:#?}");
}

#[test]
fn lex_and_parse_errors_are_both_reported() {
    let (_, diags) = parse_module("fn f() { let s = \"\\q\"; $ }");
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert_eq!(
        messages,
        ["unexpected character `$`", "unknown character escape: `q`"]
    );
}

#[test]
fn examples_parse_cleanly() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../example");
    let mut count = 0;
    for entry in std::fs::read_dir(dir).expect("read example dir") {
        let path = entry.expect("read example entry").path();
        if path.extension().is_some_and(|ext| ext == "xur") {
            let src = std::fs::read_to_string(&path).expect("read example file");
            let (module, diags) = parse_module(&src);
            assert!(diags.is_empty(), "errors in {path:?}: {diags:#?}");
            assert!(!module.items.is_empty());
            count += 1;
        }
    }
    assert!(count > 0, "no examples found in {dir}");
}
