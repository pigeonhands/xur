use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    registry
        .module("std::env")
        .function("get", |name: String| std::env::var(&name).ok())
        .function("set", |name: String, value: String| {
            // Safety: xur scripts control their own threading; this is the
            // expected trade-off for a scripting language.
            unsafe { std::env::set_var(&name, &value) };
        })
        .function("remove", |name: String| {
            unsafe { std::env::remove_var(&name) };
        })
        .function("vars", || {
            std::env::vars()
                .map(|(k, v)| vec![k, v])
                .collect::<Vec<_>>()
        })
        .function("current_dir", || {
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .map_err(|e| e.to_string())
        })
        .function("home_dir", || std::env::var("HOME").ok())
        .function("exe_path", || {
            std::env::current_exe()
                .map(|p| p.display().to_string())
                .ok()
        });
}
