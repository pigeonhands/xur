//! CLR initialization — waits for Godot to load CoreCLR, then attaches.

use xur_runtime::convert::into_value;
use xur_runtime::registry::ModuleBuilder;
use xur_runtime::{RuntimeError, Value};

use crate::stub::coreclr::{CoreClrRuntime, RUNTIME};

fn stub_diag(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/hao-diag.log")
    {
        let _ = writeln!(f, "[stub-diag] {msg}");
    }
}

pub(super) fn register(m: &mut ModuleBuilder<'_>) {
    m.raw_function("init", |cx, args| {
        let [Value::Int(major)] = args else {
            return Err(RuntimeError::new("`init` takes (major: int)"));
        };

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let major = *major as u16;

        stub_diag("about to call on_clr_init");
        hao_godot::on_clr_init(move |host| {
            stub_diag(&format!(
                "on_clr_init callback fired, attaching major={major}"
            ));
            tracing::info!("godot: attaching to CoreCLR (major={major})");
            let result = host.attach(major);
            match &result {
                Ok(_) => tracing::info!("godot: CoreCLR attach succeeded"),
                Err(e) => tracing::error!("godot: CoreCLR attach failed: {e}"),
            }
            tx.send(result).ok();
        });

        stub_diag("waiting on rx.recv()");
        let rt = rx
            .recv()
            .map_err(|_| RuntimeError::new("CLR init callback was dropped"))?
            .map_err(|e| RuntimeError::new(format!("CLR attach failed: {e}")))?;

        stub_diag("CLR attached, calling ensure_scene_tree_pump");
        hao_godot::set_runtime(rt.clone());

        #[cfg(target_arch = "x86_64")]
        if let Err(e) = hao_godot::lifecycle::ensure_scene_tree_pump(&rt) {
            // When hook_scene_tree registered an IL rewrite, the pump
            // starts asynchronously when GodotSharp.dll is opened — that
            // may not have happened yet. Don't abort.
            tracing::debug!("scene tree pump deferred (IL rewrite pending): {e}");
        }

        RUNTIME.set(rt).ok();

        let lang = cx.lang();
        let v = into_value(cx, CoreClrRuntime)?;
        Ok(Value::adt(lang.ok, vec![v]))
    });
}
