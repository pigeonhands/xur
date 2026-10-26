use xur_ast::ItemKind;

use super::show::Show;
use super::{parse_errors, parse_ok};
use xur_ast::{Module, NodeIdGen};
use xur_errors::Diag;
use xur_span::SourceMap;

use crate::parser::{Parser, parse_file};

fn parse_module(src: &str, ids: &mut NodeIdGen) -> (Module, Vec<Diag>) {
    let file = SourceMap::new().add_file("test.xur", src);
    parse_file(&file, ids)
}

fn parse_module_fresh(src: &str) -> (Module, Vec<Diag>) {
    parse_module(src, &mut NodeIdGen::default())
}

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
    let (module, diags) = parse_module_fresh(
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
    let (_, diags) = parse_module_fresh("fn f() { let s = \"\\q\"; $ }");
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
            let (module, diags) = parse_module_fresh(&src);
            assert!(diags.is_empty(), "errors in {path:?}: {diags:#?}");
            assert!(!module.items.is_empty());
            count += 1;
        }
    }
    assert!(count > 0, "no examples found in {dir}");
}

#[test]
fn spans_are_global_positions() {
    let mut map = SourceMap::new();
    map.add_file("a.xur", "fn a() {}");
    let b = map.add_file("b.xur", "\nfn b() {\n    x y\n}");
    let (module, diags) = parse_file(&b, &mut NodeIdGen::default());
    assert_eq!(module.span, b.span());
    assert_eq!(map.slice(module.items[0].span), "fn b() {\n    x y\n}");
    assert_eq!(map.describe_pos(diags[0].span), "b.xur:3:7");
}

#[test]
fn node_ids_continue_across_files() {
    let mut ids = NodeIdGen::default();
    let (first, _) = parse_module("fn a() {}", &mut ids);
    let (second, _) = parse_module("fn b() {}", &mut ids);
    assert_ne!(first.items[0].id, second.items[0].id);
}

#[test]
fn visitor_reaches_every_path() {
    use xur_ast::visit::{self, Visitor};

    #[derive(Default)]
    struct Paths(Vec<String>);
    impl Visitor for Paths {
        fn visit_path(&mut self, path: &xur_ast::Path) {
            self.0.push(path.show());
            visit::walk_path(self, path);
        }
    }

    let (module, _) = parse_module_fresh(
        "use std;\n\
         fn f<T>(x: Vec<T>) -> Result<u32, E> {\n\
             let y: u32 = g::<T>(x);\n\
             match y { Ok(v) => v.m::<W>(), A::B => |c: C| c }\n\
         }",
    );
    let mut paths = Paths::default();
    paths.visit_module(&module);
    assert_eq!(
        paths.0,
        [
            "std",
            "Vec<T>",
            "T",
            "Result<u32, E>",
            "u32",
            "E",
            "u32",
            "g<T>",
            "T",
            "x",
            "y",
            "Ok",
            "v",
            "W",
            "A::B",
            "C",
            "c"
        ]
    );
}
