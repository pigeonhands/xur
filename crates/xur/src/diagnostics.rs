use std::io::{self, IsTerminal};

use xur_errors::{Diag, Emitter};

use crate::source::SourceFile;

/// Prints `diags` for `file` to stderr, in colour when stderr is a terminal.
pub fn emit(file: &SourceFile, diags: impl IntoIterator<Item = Diag>) -> io::Result<()> {
    let stderr = io::stderr();
    let emitter = Emitter::new(&file.name, &file.text).with_color(stderr.is_terminal());
    let mut out = stderr.lock();
    for diag in diags {
        emitter.emit(&diag, &mut out)?;
    }
    Ok(())
}
