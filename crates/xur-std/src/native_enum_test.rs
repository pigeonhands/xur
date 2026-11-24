use facet::Facet;
use xur_runtime::Registry;

#[derive(Facet, Clone)]
#[repr(C)]
#[allow(dead_code)]
enum TyEnum {
    Alpha,
    Beta(String),
    Gamma(i64),
}

impl std::fmt::Display for TyEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TyEnum")
    }
}

#[derive(Facet)]
#[repr(C)]
#[allow(dead_code)]
enum ShapeEnum {
    Alpha,
    Beta(String),
    Gamma(i64),
}

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::test::native_enum", |m| {
        m.ty::<TyEnum>("TyEnum");
        m.shape_type("ShapeEnum", ShapeEnum::SHAPE);

        m.function("ty_alpha", || -> TyEnum { TyEnum::Alpha });
        m.named_function("ty_beta", &["s"], |s: String| -> TyEnum {
            TyEnum::Beta(s)
        });
        m.named_function("ty_gamma", &["n"], |n: i64| -> TyEnum {
            TyEnum::Gamma(n)
        });

        m.function("shape_alpha", || -> ShapeEnum { ShapeEnum::Alpha });
        m.named_function("shape_beta", &["s"], |s: String| -> ShapeEnum {
            ShapeEnum::Beta(s)
        });
        m.named_function("shape_gamma", &["n"], |n: i64| -> ShapeEnum {
            ShapeEnum::Gamma(n)
        });
    });
}
