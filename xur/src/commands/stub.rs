//! `xur stub` — inject the xur-stub into a running process (Linux x86-64).
//!
//! `inject` is the attach-time equivalent of running the stub under
//! `LD_PRELOAD` with `XUR_STUB_PATH`: it `dlopen`s `libxur_stub.so` into a live
//! process (via procodile's ptrace injection) and hands the injected stub a
//! `.xur` script to compile and run — the same `ctor()` → `stub_main()` path the
//! stub uses at launch.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use pewter::elf::ElfFile;
use procodile::{Access, Address, LoadMethod, Process, ProcessHandle};

use crate::cli::StubCommand;

/// The soname produced by `xur-stub/build.rs`.
const STUB_SONAME: &str = "libxur_stub.so";
/// The entry the injected stub exports (see `xur-stub/src/lib.rs`).
const STUB_ENTRY: &str = "xur_stub_run";

pub fn run(command: StubCommand) -> Result<()> {
    match command {
        StubCommand::Path => {
            println!("{}", resolve_stub()?.display());
            Ok(())
        }
        StubCommand::Inject {
            script,
            pid,
            name,
            stub,
            manual,
        } => inject(&script, pid, name.as_deref(), stub, manual),
    }
}

/// Locate `libxur_stub.so`: next to the `xur` executable (covers a dev build,
/// where the stub sits beside `xur` in `target/<profile>/`), then common system
/// library directories, then `LD_LIBRARY_PATH`.
fn resolve_stub() -> Result<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent() {
            candidates.push(dir.join(STUB_SONAME));
        }
    for base in ["/usr/lib", "/usr/local/lib", "/usr/lib/xur"] {
        candidates.push(Path::new(base).join(STUB_SONAME));
    }
    if let Some(paths) = std::env::var_os("LD_LIBRARY_PATH") {
        for dir in std::env::split_paths(&paths) {
            candidates.push(dir.join(STUB_SONAME));
        }
    }

    if let Some(found) = candidates.iter().find(|p| p.is_file()) {
        return Ok(found.clone());
    }
    bail!(
        "could not find {STUB_SONAME}; build it with `cargo build -p xur-stub` \
         or pass --stub <path>\nlooked in:\n{}",
        candidates
            .iter()
            .map(|p| format!("  {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// Injection relies on ptrace; on a permission error add the usual remedies so
/// the user isn't left guessing at a bare "Permission denied".
fn with_ptrace_hint(e: anyhow::Error) -> anyhow::Error {
    let msg = format!("{e:#}");
    if msg.contains("Permission denied") || msg.contains("os error 13") {
        e.context(
            "ptrace is required to inject into a running process; set \
             /proc/sys/kernel/yama/ptrace_scope to 0, run with sudo, or grant the \
             xur binary CAP_SYS_PTRACE",
        )
    } else {
        e
    }
}

/// Open the target process named by `--pid` or `--name`.
fn open_target(pid: Option<u32>, name: Option<&str>) -> Result<ProcessHandle> {
    match (pid, name) {
        (Some(pid), _) => {
            ProcessHandle::open(pid, Access::All).with_context(|| format!("opening pid {pid}"))
        }
        (None, Some(name)) => {
            let matches: Vec<Process> = Process::by_name(name)
                .with_context(|| format!("searching for process {name:?}"))?
                .collect();
            match matches.as_slice() {
                [] => bail!("no running process named {name:?}"),
                [p] => p
                    .open(Access::All)
                    .with_context(|| format!("opening {name:?} (pid {})", p.pid())),
                many => bail!(
                    "{} processes named {name:?}: pids {}; select one with --pid",
                    many.len(),
                    many.iter()
                        .map(|p| p.pid().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
        (None, None) => bail!("specify a target with --pid <PID> or --name <NAME>"),
    }
}

/// Resolve the runtime address of `xur_stub_run` in the target: the loaded
/// module base plus the symbol's offset within the `.so`.
fn resolve_entry(stub: &Path, base: Address) -> Result<Address> {
    let bytes = std::fs::read(stub).with_context(|| format!("reading {}", stub.display()))?;
    let elf = ElfFile::parse(&bytes).map_err(|e| anyhow!("parsing {}: {e}", stub.display()))?;
    let sym = elf
        .find_symbol(STUB_ENTRY)
        .map_err(|e| anyhow!("reading symbols from {}: {e}", stub.display()))?
        .ok_or_else(|| anyhow!("{} does not export {STUB_ENTRY}", stub.display()))?;
    Ok(Address::from(base.get() + sym.value as usize))
}

fn inject(
    script: &Path,
    pid: Option<u32>,
    name: Option<&str>,
    stub: Option<PathBuf>,
    manual: bool,
) -> Result<()> {
    // Absolute path: the stub runs inside the target, whose CWD may differ.
    let script = script
        .canonicalize()
        .with_context(|| format!("script {}", script.display()))?;
    let script_str = script
        .to_str()
        .ok_or_else(|| anyhow!("script path is not valid UTF-8: {}", script.display()))?;

    let stub = match stub {
        Some(p) => p,
        None => resolve_stub()?,
    };
    if !stub.is_file() {
        bail!("stub not found: {}", stub.display());
    }

    let handle = open_target(pid, name).map_err(with_ptrace_hint)?;

    // 1. Load the stub into the target (dlopen, or a manual ELF map). Either way
    //    its #[ctor] init() fires but auto-runs nothing (no XUR_STUB_PATH here),
    //    so it returns promptly, and we get back the loaded base address.
    let method = if manual {
        LoadMethod::ManualMap
    } else {
        LoadMethod::Loader
    };
    let module = handle
        .load_library(&stub, method)
        .with_context(|| format!("loading {} into the target", stub.display()))
        .map_err(with_ptrace_hint)?;

    // 2. Resolve the injected entry point in the loaded module.
    let entry = resolve_entry(&stub, module.base)?;

    // 3. Write the NUL-terminated script path into the target.
    let mut path_bytes = script_str.as_bytes().to_vec();
    path_bytes.push(0);
    let arg = handle
        .inject_bytes(&path_bytes)
        .context("writing the script path into the target")?;

    // 4. Run xur_stub_run(path) on a detached thread in the target.
    handle
        .execute_remote_detached(entry, arg.get() as u64)
        .context("starting xur_stub_run in the target")?;

    println!("injected {} — running {}", stub.display(), script.display());
    println!("logs: /tmp/xur-stub.log");
    Ok(())
}
