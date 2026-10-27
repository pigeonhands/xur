use std::io::{self, IsTerminal};

use xur_errors::{Diag, Emitter};
use xur_span::SourceMap;

/// Prints `diags` to stderr, in colour when stderr is a terminal.
pub fn emit(source_map: &SourceMap, diags: impl IntoIterator<Item = Diag>) -> io::Result<()> {
    let stderr = io::stderr();
    let emitter = Emitter::new(source_map).with_color(stderr.is_terminal());
    let mut out = stderr.lock();
    for diag in diags {
        emitter.emit(&diag, &mut out)?;
    }
    Ok(())
}
