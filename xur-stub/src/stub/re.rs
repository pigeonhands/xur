use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    #[cfg(feature = "il2cpp-runtime")]
    registry.lazy_module("std::stub::re::il2cpp_runtime", |m| {
        let registry = m.registry();
        xur_std::re::walk::register(
            registry,
            m,
            hao_il2cpp_runtime::reflect::roots(),
            |shape| {
                let path = shape.module_path?;
                if path.starts_with("hao_il2cpp_runtime") {
                    Some("")
                } else {
                    None
                }
            },
            &[],
        );
    });

    #[cfg(feature = "il2cpp-unity")]
    registry.lazy_module("std::stub::re::unity", |m| {
        let registry = m.registry();
        xur_std::re::walk::register(
            registry,
            m,
            hao_il2cpp_unity::reflect::roots(),
            |shape| {
                let path = shape.module_path?;
                if path.starts_with("hao_il2cpp_unity") {
                    Some("")
                } else {
                    None
                }
            },
            &[],
        );
    });

    #[cfg(feature = "godot")]
    registry.lazy_module("std::stub::re::godot", |m| {
        let registry = m.registry();
        xur_std::re::walk::register(
            registry,
            m,
            &hao_godot::reflect::roots(),
            |shape| {
                let path = shape.module_path?;
                if path.starts_with("hao_runtime") {
                    Some("")
                } else {
                    None
                }
            },
            &[],
        );
    });
}
