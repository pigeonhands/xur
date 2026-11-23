//! `std::stub::godot` — Godot engine init and lifecycle for LD_PRELOAD game mods.
//!
//! Hooks the game's CoreCLR initialization and exposes it as a
//! `coreclr::Runtime`. Provides lifecycle pump setup, main-thread dispatch,
//! and scene tree traversal for safely calling managed methods from other threads.
//!
//! ```text
//! use std::stub::godot;
//!
//! let rt = godot::init(9)?;
//! godot::hook_scene_tree();  // hooks SceneTree._Process — works for any game
//! godot::dispatch(|| { /* runs on game thread */ });
//! let root = godot::root();  // root Window node
//! ```

mod dispatch;
mod init;
mod lifecycle;
mod scene;

use xur_runtime::Registry;

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::stub::godot", |m| {
        init::register(m);
        lifecycle::register(m);
        dispatch::register(m);
        scene::register(m);
    });
}

pub(crate) fn detect_lib_name() -> String {
    #[cfg(unix)]
    {
        // CoreCLR's P/Invoke resolver calls dlopen() with this name.
        // A path containing '/' is treated as a filesystem path (resolved
        // relative to cwd), which fails when the game's cwd differs from
        // where xur-stub was built.  A bare filename like "libxur_stub.so"
        // is searched by soname among already-loaded libraries — since
        // LD_PRELOAD already loaded us, dlopen finds us immediately.
        if let Ok(preload) = std::env::var("LD_PRELOAD") {
            if let Some(path) = preload.split(':').find(|p| p.contains("xur_stub")) {
                if let Some(name) = path.rsplit('/').next() {
                    if !name.is_empty() {
                        return name.to_string();
                    }
                }
                return path.to_string();
            }
        }
        return "libxur_stub.so".into();
    }
    #[cfg(windows)]
    return "xur_stub.dll".into();
}
