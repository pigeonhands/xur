//! `xur run`: running a program.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use anyhow::{Result, bail};
use xur_resolve::Program;
use xur_runtime::Registry;
use xur_span::MemoryFileLoader;
use xur_vm::Vm;

use crate::diagnostics;
use crate::load::{fail_on_errors, load_program};

/// The interpreter's stack. Deep recursion in a Xur program recurses in the
/// interpreter too.
const STACK_SIZE: usize = 256 << 20;

/// Loads the program rooted at `path` and runs its `main` function.
pub fn run(path: &Path, tracing: bool) -> Result<()> {
    let (program, registry, diags) = load_program(path)?;
    run_program(program, registry, &path.display().to_string(), diags, tracing)
}

/// Runs an in-memory source as if it were at `path`.
pub fn run_source(path: PathBuf, src: String, tracing: bool) -> Result<()> {
    let registry = Arc::new(xur_std::registry());
    let natives = Box::new(Arc::clone(&registry));
    let name = path.display().to_string();
    let loader = Box::new(MemoryFileLoader::new([(name.as_str(), src.as_str())]));
    let (program, diags) = Program::load(&path, loader, natives)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    run_program(program, registry, &name, diags, tracing)
}

fn run_program(
    program: Program,
    registry: Arc<Registry>,
    root: &str,
    diags: Vec<xur_errors::Diag>,
    tracing: bool,
) -> Result<()> {
    fail_on_errors(&program.krate.source_map, root, diags, "compile")?;

    let vm = Vm::new(program, registry, Box::new(io::stdout()));
    vm.set_tracing(tracing);
    let result = thread::scope(|scope| {
        thread::Builder::new()
            .name("main".to_owned())
            .stack_size(STACK_SIZE)
            .spawn_scoped(scope, || vm.run_main())
            .map(|handle| handle.join())
    });
    let result = match result {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => bail!("the interpreter panicked"),
        Err(error) => bail!("couldn't start the interpreter: {error}"),
    };

    vm.flush_stdout();

    let program = vm.program();
    let source_map = &program.krate.source_map;
    diagnostics::emit(source_map, vm.take_diagnostics())?;
    if let Err(error) = result {
        match error.span {
            Some(span) => diagnostics::emit(source_map, [error.into_diag(span)])?,
            None => {
                eprintln!("Error: {}", error.message);
                for note in &error.notes {
                    eprintln!("  {note}");
                }
            }
        }
        bail!("`{root}` stopped with a runtime error");
    }
    Ok(())
}
