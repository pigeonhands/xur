use super::{resolve_files, resolve_src};

const SHAPE: &str = "pub enum Shape { Circle(f64) Rect(f64, f64) }\n\
impl Shape {\n\
    pub fn square(side: f64) -> Self { Shape::Rect(side, side) }\n\
    pub fn area(self) -> f64 { match self { Shape::Circle(r) => r * r, Shape::Rect(w, h) => w * h } }\n\
    fn helper(self) {}\n\
}";

#[test]
fn associated_functions_are_paths_on_the_type() {
    let resolved = resolve_src(&format!(
        "{SHAPE}\nfn f() {{ Shape::square(2.0).area(); Shape::area(Shape::Circle(1.0)); }}"
    ));
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("Shape::square"), "function Shape::square");
    assert_eq!(resolved.res_of("Shape::area"), "function Shape::area");
}

#[test]
fn self_and_self_type() {
    let resolved = resolve_src(SHAPE);
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("Self"), "enum Shape");
    assert_eq!(
        resolved.res_of_nth("self", 1),
        "local self at main.xur:4:13"
    );
}

#[test]
fn impls_on_structs() {
    let resolved = resolve_src(
        "struct Point(f64, f64);\n\
         impl Point { fn origin() -> Self { Point(0.0, 0.0) } }\n\
         fn f() { Point::origin(); }",
    );
    resolved.assert_no_errors();
    assert_eq!(resolved.res_of("Point::origin"), "function Point::origin");
}

#[test]
fn impl_generics_are_visible_in_methods() {
    resolve_src("struct W<T>(T);\nimpl<T> W<T> { fn get(self) -> T { self } }").assert_no_errors();
}

#[test]
fn private_methods_across_modules() {
    let resolved = resolve_files(&[
        (
            "main.xur",
            "const shapes = std::import(\"shapes.xur\");\nfn f() { shapes::Shape::square(1.0); shapes::Shape::helper(); }",
        ),
        ("shapes.xur", SHAPE),
    ]);
    assert_eq!(resolved.errors, ["function `helper` is private"]);
}

#[test]
fn impl_errors() {
    assert_eq!(
        resolve_src("impl u32 { fn f() {} }").errors,
        ["`impl` blocks can only add functions to structs and enums"]
    );
    assert_eq!(
        resolve_src("impl Nope { fn f() {} }").errors,
        ["cannot find type `Nope` in this scope"]
    );
    assert_eq!(
        resolve_src("enum E { A }\nimpl E { fn A() {} }").errors,
        ["the name `A` is defined multiple times"]
    );
    assert_eq!(
        resolve_src("struct S;\nfn f() { impl S {} }").errors,
        ["`impl` blocks are only allowed at the top level of a module"]
    );
}
