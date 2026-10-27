use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use xur_resolve::Program;
use xur_runtime::Registry;
use xur_span::RealFileLoader;
use xur_vm::Vm;

#[cfg(feature = "egui")]
mod egui_module;
mod tracing_module;

const STACK_SIZE: usize = 256 << 20;
const LOG_PATH: &str = "/tmp/xur-stub.log";

fn init_logging() {
    let _ = std::fs::write(LOG_PATH, b"");
}

fn log(msg: std::fmt::Arguments<'_>) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(LOG_PATH) {
        let _ = writeln!(f, "[xur-stub] {msg}");
    }
    tracing::info!("{msg}");
    eprintln!("[xur-stub] {msg}");
}

/// Compiled script ready to run.
struct Script {
    vm: Vm,
    path: PathBuf,
}

#[ctor::ctor]
fn init() {
    xur_std::set_invoked_by("stub");
    init_logging();
    log(format_args!("init"));

    // Compile on a big-stack thread, but block until ctor() functions
    // have run — they must complete before the game's main() starts
    // (e.g. to install Il2Cpp hooks via egui::init()).
    let (scripts_tx, scripts_rx) = std::sync::mpsc::sync_channel::<Vec<Script>>(1);

    thread::Builder::new()
        .name("xur-stub".into())
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let scripts = match compile_scripts() {
                Ok(s) => {
                    log(format_args!("compiled {} script(s)", s.len()));
                    s
                }
                Err(e) => {
                    log(format_args!("{e}"));
                    scripts_tx.send(Vec::new()).ok();
                    return;
                }
            };

            for script in &scripts {
                if let Some(Err(e)) = script.vm.run_fn("ctor") {
                    log(format_args!("{}: ctor: {}", script.path.display(), e.message));
                }
            }

            scripts_tx.send(scripts).ok();
        })
        .expect("xur-stub: failed to spawn init thread");

    let scripts = scripts_rx.recv().unwrap_or_default();

    // Run main() on background threads — these can block forever (e.g.
    // egui overlays) without preventing the game from starting.
    for script in scripts {
        log(format_args!("starting {}", script.path.display()));
        thread::Builder::new()
            .name(format!("xur:{}", script.path.display()))
            .stack_size(STACK_SIZE)
            .spawn(move || {
                if let Err(e) = run_main(&script) {
                    log(format_args!("{}: {e}", script.path.display()));
                }
            })
            .ok();
    }
}

fn compile_scripts() -> Result<Vec<Script>, String> {
    let dir = script_dir()?;
    let paths = find_scripts(&dir)?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut scripts = Vec::new();
    for path in paths {
        match compile_script(&path) {
            Ok(vm) => scripts.push(Script { vm, path }),
            Err(e) => log(format_args!("{}: {e}", path.display())),
        }
    }
    Ok(scripts)
}

fn compile_script(path: &Path) -> Result<Vm, String> {
    let registry = make_registry();
    let natives = Box::new(Arc::clone(&registry));
    let (program, diags) =
        Program::load(path, Box::new(RealFileLoader), natives).map_err(|e| e.to_string())?;

    if !diags.is_empty() {
        for d in &diags {
            log(format_args!("{}: {d}", path.display()));
        }
        return Err(format!("{} compile error(s)", diags.len()));
    }

    Ok(Vm::new(program, registry, Box::new(io::stderr())))
}

fn run_main(script: &Script) -> Result<(), String> {
    let result = thread::scope(|scope| {
        thread::Builder::new()
            .name(script.path.display().to_string())
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || {
                // Prefer stub_main() so a script can serve as both a CLI
                // program and a stub payload.
                match script.vm.run_fn("stub_main") {
                    Some(r) => r,
                    None => script.vm.run_main(),
                }
            })
            .map(|h| h.join())
    });

    match result {
        Ok(Ok(Ok(_))) => Ok(()),
        Ok(Ok(Err(e))) => Err(format!("runtime error: {}", e.message)),
        Ok(Err(_)) => Err("interpreter panicked".into()),
        Err(e) => Err(format!("failed to start: {e}")),
    }
}

fn script_dir() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("XUR_STUB_PATH") {
        let p = PathBuf::from(path);
        if p.is_dir() {
            return Ok(p);
        }
        if p.is_file() {
            return Ok(p);
        }
        return Err(format!("XUR_STUB_PATH={}: not found", p.display()));
    }
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let dir = exe.parent().ok_or("cannot determine exe directory")?;
    let xur_dir = dir.join("xur");
    if xur_dir.is_dir() {
        Ok(xur_dir)
    } else {
        Err(format!("no scripts: {} does not exist", xur_dir.display()))
    }
}

fn find_scripts(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let entries =
        std::fs::read_dir(path).map_err(|e| format!("read_dir {}: {e}", path.display()))?;
    let mut scripts: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "xur"))
        .collect();
    scripts.sort();
    Ok(scripts)
}

fn register_game_mods(registry: &Registry) {
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

    #[cfg(feature = "il2cpp-unity")]
    registry.lazy_module("std::stub::re::unity::time", |m| {
        use hao_il2cpp_unity::time;
        use xur_runtime::{RuntimeError, Value};

        m.raw_function("scale", |_cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`scale` takes no arguments"));
            }
            let scale = time::scale().unwrap_or(1.0);
            Ok(Value::Float(scale as f64))
        });

        m.raw_function("set_scale", |_cx, args| {
            let [Value::Float(scale)] = args else {
                return Err(RuntimeError::new("`set_scale` takes a float"));
            };
            let _ = time::set_scale(*scale as f32);
            Ok(Value::Unit)
        });

        m.raw_function("delta", |_cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`delta` takes no arguments"));
            }
            let dt = time::delta().unwrap_or(0.0);
            Ok(Value::Float(dt as f64))
        });

        m.raw_function("frame_count", |_cx, args| {
            if !args.is_empty() {
                return Err(RuntimeError::new("`frame_count` takes no arguments"));
            }
            let count = time::frame_count().unwrap_or(0);
            Ok(Value::Int(count as i64))
        });
    });

    #[cfg(feature = "egui")]
    registry.lazy_module("std::stub::re::unity::egui", |m| {
        let registry = m.registry();
        egui_module::register(registry, m);
    });

    registry.lazy_module("std::stub::tracing", |m| {
        tracing_module::register(m);
    });
}

fn make_registry() -> Arc<Registry> {
    Arc::new({
        let r = xur_std::registry();
        register_game_mods(&r);
        r
    })
}
