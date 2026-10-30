use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use xur_resolve::Program;
use xur_runtime::Registry;
use xur_span::RealFileLoader;
use xur_vm::Vm;

mod stub;


const STACK_SIZE: usize = 256 << 20;
const LOG_PATH: &str = "/tmp/xur-stub.log";

fn init_logging() {
    let _ = std::fs::write(LOG_PATH, b"");
}

#[cfg(unix)]
fn install_crash_handler() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = crash_signal_handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
        libc::sigaction(libc::SIGSEGV, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGBUS, &sa, std::ptr::null_mut());
    }
}

#[cfg(unix)]
unsafe extern "C" fn crash_signal_handler(
    sig: libc::c_int,
    info: *mut libc::siginfo_t,
    ctx: *mut libc::c_void,
) {
    use std::io::Write;
    let rip = if ctx.is_null() {
        0u64
    } else {
        let uctx = ctx as *const libc::ucontext_t;
        (*uctx).uc_mcontext.gregs[libc::REG_RIP as usize] as u64
    };
    let fault_addr = if info.is_null() {
        0u64
    } else {
        (*info).si_addr() as u64
    };
    let sig_name = if sig == libc::SIGSEGV { "SIGSEGV" } else { "SIGBUS" };
    let msg = format!(
        "\n[xur-stub] CRASH: {} at rip=0x{:x} fault_addr=0x{:x}\n",
        sig_name, rip, fault_addr,
    );
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_PATH)
        .and_then(|mut f| f.write_all(msg.as_bytes()));
    // Also write to stderr for lldb/terminal visibility.
    let _ = libc::write(2, msg.as_ptr() as *const _, msg.len());
    // Re-raise with default handler.
    libc::signal(sig, libc::SIG_DFL);
    libc::raise(sig);
}

#[cfg(not(unix))]
fn install_crash_handler() {}

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
    install_crash_handler();
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
        Program::load(path, Box::new(RealFileLoader), natives, Some(xur_std::PRELUDE))
            .map_err(|e| e.to_string())?;

    if !diags.is_empty() {
        for d in &diags {
            log(format_args!("{}: {d}", path.display()));
        }
        return Err(format!("{} compile error(s)", diags.len()));
    }

    Ok(Vm::new(program, registry, Box::new(io::stderr())))
}

fn run_main(script: &Script) -> Result<(), String> {
    // Prefer stub_main() so a script can serve as both a CLI
    // program and a stub payload.
    let result = match script.vm.run_fn("stub_main") {
        Some(r) => r,
        None => script.vm.run_main(),
    };
    result
        .map(|_| ())
        .map_err(|e| format!("runtime error: {}", e.message))
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


fn make_registry() -> Arc<Registry> {
    Arc::new({
        let r = xur_std::registry();
        stub::register(&r);
        r
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use xur_resolve::Program;
    use xur_span::MemoryFileLoader;

    use super::make_registry;

    fn compile(name: &str, src: &str) {
        let registry = make_registry();
        let loader = MemoryFileLoader::new(std::iter::once((name, src)));
        let (_, diags) = Program::load(
            Path::new(name),
            Box::new(loader),
            Box::new(Arc::clone(&registry)),
            Some(xur_std::PRELUDE),
        )
        .unwrap();
        let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
        assert!(diags.is_empty(), "{name}: {messages:#?}");
    }

    #[test]
    fn examples_compile() {
        compile("hello.xur", include_str!("../example/hello.xur"));
        compile("tracing.xur", include_str!("../example/tracing.xur"));
        compile("hooking.xur", include_str!("../example/hooking.xur"));
        compile("game_inspector.xur", include_str!("../example/game_inspector.xur"));
        compile("godot.xur", include_str!("../example/godot.xur"));
        compile("sts2.xur", include_str!("../example/sts2.xur"));
        compile("megabonk_discover.xur", include_str!("../example/megabonk_discover.xur"));
    }

    #[test]
    #[cfg(feature = "egui")]
    fn egui_examples_compile() {
        compile("egui_overlay.xur", include_str!("../example/egui_overlay.xur"));
        compile("megabonk.xur", include_str!("../example/megabonk.xur"));
    }

}
