use facet::Facet;

use super::*;
use crate::testing::{TestCx, fn_id};

#[derive(Facet, Debug, Clone, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry
        .module("geo::shapes")
        .function("double", |x: i64| x * 2)
        .function("describe", |name: String, n: u8| format!("{name}#{n}"));
    registry
        .module("geo")
        .ty::<Point>("Point")
        .function("new", |x: f64, y: f64| Point { x, y })
        .method("norm", |p: &Point| p.x.hypot(p.y))
        .method_mut("scale", |p: &mut Point, k: f64| {
            p.x *= k;
            p.y *= k;
        });
    registry
}

#[test]
fn decls_describe_the_module_tree() {
    let decls = registry().decls();
    assert_eq!(decls.len(), 1);
    let geo = &decls[0];
    assert_eq!(geo.name, "geo");
    assert_eq!(geo.modules[0].name, "shapes");
    assert_eq!(geo.modules[0].functions[1].name, "describe");
    assert_eq!(geo.types[0].name, "Point");
    assert_eq!(geo.types[0].functions[1].name, "norm");
    assert_eq!(geo.types[0].functions[2].name, "scale");
}

#[test]
fn typed_functions_convert_arguments_and_results() {
    let registry = registry();
    let mut cx = TestCx::new(&registry);
    let double = fn_id(&registry, "geo::shapes::double");
    assert_eq!(cx.call(double, &[Value::Int(21)]), Ok(Value::Int(42)));
    let describe = fn_id(&registry, "geo::shapes::describe");
    assert_eq!(
        cx.call(describe, &[Value::str("p"), Value::Int(3)]),
        Ok(Value::str("p#3"))
    );
}

#[test]
fn signatures_record_shapes() {
    let registry = registry();
    let signature = registry
        .function(fn_id(&registry, "geo::shapes::describe"))
        .signature
        .clone()
        .unwrap();
    assert_eq!(signature.params, [String::SHAPE, u8::SHAPE]);
    assert_eq!(signature.ret, String::SHAPE);
}

fn new_point(cx: &mut TestCx<'_>, x: f64, y: f64) -> Value {
    let new = fn_id(cx.registry, "geo::Point::new");
    cx.call(new, &[Value::Float(x), Value::Float(y)]).unwrap()
}

#[test]
fn registered_types_are_native_objects() {
    let registry = registry();
    let mut cx = TestCx::new(&registry);
    let point = new_point(&mut cx, 3.0, 4.0);
    let Value::Native(object) = &point else {
        panic!("expected a native object, found {point:?}");
    };
    assert_eq!(*object.borrow::<Point>().unwrap(), Point { x: 3.0, y: 4.0 });
    assert_eq!(
        cx.call(fn_id(&registry, "geo::Point::norm"), &[point]),
        Ok(Value::Float(5.0))
    );
}

#[test]
fn mutating_methods_change_the_object_in_place() {
    let registry = registry();
    let mut cx = TestCx::new(&registry);
    let point = new_point(&mut cx, 1.0, 2.0);
    let alias = point.clone();
    let scale = fn_id(&registry, "geo::Point::scale");
    assert_eq!(cx.call(scale, &[point, Value::Float(3.0)]), Ok(Value::Unit));
    let Value::Native(object) = &alias else {
        unreachable!()
    };
    assert_eq!(*object.borrow::<Point>().unwrap(), Point { x: 3.0, y: 6.0 });
}

#[test]
fn native_fields_display_and_equality() {
    let registry = registry();
    let mut cx = TestCx::new(&registry);
    let (Value::Native(a), Value::Native(b)) =
        (new_point(&mut cx, 1.0, 2.0), new_point(&mut cx, 1.0, 2.0))
    else {
        unreachable!()
    };
    assert_eq!(registry.field(&cx, &a, "y"), Some(Ok(Value::Float(2.0))));
    assert_eq!(registry.field(&cx, &a, "z"), None);
    assert_eq!(registry.display(&a), "Point { x: 1.0, y: 2.0 }");
    assert!(registry.native_eq(&a, &b));
}

#[test]
fn bad_calls_are_runtime_errors() {
    let registry = registry();
    let mut cx = TestCx::new(&registry);
    let double = fn_id(&registry, "geo::shapes::double");
    assert_eq!(
        cx.call(double, &[]).unwrap_err().message,
        "expected 1 argument(s), found 0"
    );
    assert_eq!(
        cx.call(double, &[Value::str("x")]).unwrap_err().message,
        "expected `i64`, found string"
    );
}
