use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::re")
        .function("demangle", |name: String| rekit::demangle::demangle(&name))
        .function("demangle_itanium", |name: String| {
            rekit::demangle::demangle_itanium(&name)
        })
        .function("demangle_msvc", |name: String| {
            rekit::demangle::demangle_msvc(&name)
        });
}
