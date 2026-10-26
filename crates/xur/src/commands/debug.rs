//! `xur debug ...`: dump the output of individual compiler stages.

use std::io::{self, BufWriter, Write};
use std::path::Path;

use anyhow::{Result, bail};
use xur_parser::{Lexer, LineIndex, Token};

use crate::cli::DebugCommand;
use crate::diagnostics;
use crate::source::SourceFile;

pub fn run(command: DebugCommand) -> Result<()> {
    match command {
        DebugCommand::Tokens { file } => tokens(&file),
    }
}

/// Prints one row per token: position, kind, and the source text it covers.
fn tokens(path: &Path) -> Result<()> {
    let file = SourceFile::load(path)?;
    let (tokens, errors) = Lexer::new(&file.text).tokenise();

    let mut out = BufWriter::new(io::stdout().lock());
    ignore_broken_pipe(write_token_table(&mut out, &file.text, &tokens))?;

    if !errors.is_empty() {
        diagnostics::report_lex_errors(&file, &errors)?;
        bail!(
            "could not lex `{}` due to {} error(s)",
            file.name,
            errors.len()
        );
    }
    Ok(())
}

fn write_token_table(out: &mut impl Write, src: &str, tokens: &[Token<'_>]) -> io::Result<()> {
    let lines = LineIndex::new(src);
    for token in tokens {
        let position = lines.line_col(token.span.start).to_string();
        let kind = token.kind.to_string();
        writeln!(out, "{position:<8} {kind:<10} {}", token.span.slice(src))?;
    }
    out.flush()
}

/// Treats a closed stdout (e.g. `xur debug tokens f.xur | head`) as success.
fn ignore_broken_pipe(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_table_rows() {
        let src = "let x =\n  \"hi\";";
        let (tokens, _) = Lexer::new(src).tokenise();
        let mut out = Vec::new();
        write_token_table(&mut out, src, &tokens).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "1:1      let        let\n\
             1:5      <ident>    x\n\
             1:7      =          =\n\
             2:3      <string>   \"hi\"\n\
             2:7      ;          ;\n\
             2:8      <eof>      \n"
        );
    }
}
