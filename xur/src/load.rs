//! Loading programs with the standard library.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use xur_errors::Diag;
use xur_resolve::Program;
use xur_runtime::Registry;
use xur_span::{RealFileLoader, SourceMap};

use crate::diagnostics;

/// Loads and resolves the program rooted at `path` with `std`, returning
/// its diagnostics rather than failing on them.
pub fn load_program(path: &Path) -> Result<(Program, Arc<Registry>, Vec<Diag>)> {
    let registry = Arc::new(xur_std::registry());
    let natives = Box::new(Arc::clone(&registry));
    let (program, diags) = Program::load(path, Box::new(RealFileLoader), natives, Some(xur_std::PRELUDE))
        .with_context(|| format!("failed to read `{}`", path.display()))?;
    Ok((program, registry, diags))
}

/// Reports `diags`, failing if there were any. `root` names the file the
/// command was run on.
pub fn fail_on_errors(
    source_map: &SourceMap,
    root: &str,
    diags: Vec<Diag>,
    stage: &str,
) -> Result<()> {
    if diags.is_empty() {
        return Ok(());
    }
    let count = diags.len();
    diagnostics::emit(source_map, diags)?;
    bail!("could not {stage} `{root}` due to {count} error(s)")
}
