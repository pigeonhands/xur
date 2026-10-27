//! `xur run`: running a program.

use std::io;
use std::path::Path;
use std::thread;

use anyhow::{Result, bail};
use xur_vm::Vm;

use crate::diagnostics;
use crate::load::{fail_on_errors, load_program};

/// The interpreter's stack. Deep recursion in a Xur program recurses in the
/// interpreter too.
const STACK_SIZE: usize = 256 << 20;

/// Loads the program rooted at `path` and runs its `main` function.
pub fn run(path: &Path) -> Result<()> {
    let (program, registry, diags) = load_program(path)?;
    let root = path.display().to_string();
    fail_on_errors(&program.krate.source_map, &root, diags, "compile")?;

    let vm = Vm::new(program, registry, Box::new(io::stdout()));
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

    let program = vm.program();
    let source_map = &program.krate.source_map;
    diagnostics::emit(source_map, vm.take_diagnostics())?;
    if let Err(error) = result {
        let message = error.message.clone();
        match error.span {
            Some(span) => diagnostics::emit(source_map, [error.into_diag(span)])?,
            None => bail!("{message}"),
        }
        bail!("`{root}` stopped with a runtime error");
    }
    Ok(())
}
