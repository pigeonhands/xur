//! `xur debug ...`: dump the output of individual compiler stages.

use std::io::{self, BufWriter, Write};
use std::path::Path;

use anyhow::{Result, bail};
use xur_ast::NodeIdGen;
use xur_errors::Diag;
use xur_parser::{Lexer, Token};
use xur_span::LineIndex;

use crate::cli::DebugCommand;
use crate::diagnostics;
use crate::source::SourceFile;

pub fn run(command: DebugCommand) -> Result<()> {
    match command {
        DebugCommand::Tokens { file } => tokens(&file),
        DebugCommand::Ast { file } => ast(&file),
    }
}

/// Prints one row per token: position, kind, and the source text it covers.
fn tokens(path: &Path) -> Result<()> {
    let file = SourceFile::load(path)?;
    let (tokens, errors) = Lexer::new(&file.text).tokenise();

    let mut out = BufWriter::new(io::stdout().lock());
    ignore_broken_pipe(write_token_table(&mut out, &file.text, &tokens))?;

    let diags: Vec<_> = errors.into_iter().map(Diag::from).collect();
    fail_on_errors(&file, diags, "lex")
}

/// Prints the module's syntax tree, like rustc's `-Zunpretty=ast-tree`.
/// The tree is printed even when there are errors, with `Err` nodes where
/// parsing failed.
fn ast(path: &Path) -> Result<()> {
    let file = SourceFile::load(path)?;
    let (module, diags) = xur_parser::parse_module(&file.text, &mut NodeIdGen::default());

    let mut out = BufWriter::new(io::stdout().lock());
    ignore_broken_pipe(writeln!(out, "{module:#?}").and_then(|()| out.flush()))?;

    fail_on_errors(&file, diags, "parse")
}

/// Reports `diags`, failing if there were any.
fn fail_on_errors(file: &SourceFile, diags: Vec<Diag>, stage: &str) -> Result<()> {
    if diags.is_empty() {
        return Ok(());
    }
    let count = diags.len();
    diagnostics::emit(file, diags)?;
    bail!("could not {stage} `{}` due to {count} error(s)", file.name)
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
