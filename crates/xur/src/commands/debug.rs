//! `xur debug ...`: dump the output of individual compiler stages.

use std::io::{self, BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use xur_ast::NodeIdGen;
use xur_errors::Diag;
use xur_parser::{Lexer, Token};
use xur_resolve::{Crate, Program, Res, Resolutions};
use xur_span::{RealFileLoader, SourceFile, SourceMap};

use crate::cli::DebugCommand;
use crate::{diagnostics, source};

pub fn run(command: DebugCommand) -> Result<()> {
    match command {
        DebugCommand::Tokens { file } => tokens(&file),
        DebugCommand::Ast { file } => ast(&file),
        DebugCommand::Resolve { file } => resolve(&file),
    }
}

/// Prints one row per token: position, kind, and the source text it covers.
fn tokens(path: &Path) -> Result<()> {
    let mut source_map = SourceMap::new();
    let file = source::load(&mut source_map, path)?;
    let (tokens, errors) = Lexer::for_file(&file).tokenise();

    let mut out = BufWriter::new(io::stdout().lock());
    ignore_broken_pipe(write_token_table(&mut out, &file, &tokens))?;

    let diags: Vec<_> = errors.into_iter().map(Diag::from).collect();
    fail_on_errors(&source_map, &file.name, diags, "lex")
}

/// Prints the module's syntax tree, like rustc's `-Zunpretty=ast-tree`.
/// The tree is printed even when there are errors, with `Err` nodes where
/// parsing failed.
fn ast(path: &Path) -> Result<()> {
    let mut source_map = SourceMap::new();
    let file = source::load(&mut source_map, path)?;
    let (module, diags) = xur_parser::parse_file(&file, &mut NodeIdGen::default());

    let mut out = BufWriter::new(io::stdout().lock());
    ignore_broken_pipe(writeln!(out, "{module:#?}").and_then(|()| out.flush()))?;

    fail_on_errors(&source_map, &file.name, diags, "parse")
}

/// Loads the program rooted at `path` and prints, for each module, every
/// path and identifier pattern with what it resolves to.
fn resolve(path: &Path) -> Result<()> {
    let (program, diags) = Program::load(path, Box::new(RealFileLoader))
        .with_context(|| format!("failed to read `{}`", path.display()))?;
    let krate = &program.krate;

    let mut out = BufWriter::new(io::stdout().lock());
    ignore_broken_pipe(write_resolutions(&mut out, krate, program.resolutions()))?;

    let root = &krate.module(krate.root).path.display().to_string();
    fail_on_errors(&krate.source_map, root, diags, "resolve")
}

/// Reports `diags`, failing if there were any. `root` names the file the
/// command was run on.
fn fail_on_errors(source_map: &SourceMap, root: &str, diags: Vec<Diag>, stage: &str) -> Result<()> {
    if diags.is_empty() {
        return Ok(());
    }
    let count = diags.len();
    diagnostics::emit(source_map, diags)?;
    bail!("could not {stage} `{root}` due to {count} error(s)")
}

/// One row per name, module by module (the prelude is left out):
/// position, the name as written, and what it resolves to.
fn write_resolutions(
    out: &mut impl Write,
    krate: &Crate,
    resolutions: &Resolutions,
) -> io::Result<()> {
    let source_map = &krate.source_map;
    let prelude = krate.module(krate.prelude);
    for module in krate.modules.iter().filter(|m| !std::ptr::eq(*m, prelude)) {
        for name in xur_resolve::names(module.ast) {
            let position = source_map.describe_pos(name.span);
            let text = source_map.slice(name.span);
            let meaning = match resolutions.res.get(&name.id) {
                Some(&Res::Local(id)) if id == name.id => "new local".to_owned(),
                Some(&res) => resolutions.describe(res, source_map),
                None => "unresolved".to_owned(),
            };
            writeln!(out, "{position:<22} {text:<28} {meaning}")?;
        }
    }
    out.flush()
}

fn write_token_table(
    out: &mut impl Write,
    file: &SourceFile,
    tokens: &[Token<'_>],
) -> io::Result<()> {
    for token in tokens {
        let position = file.line_col(token.span.start).to_string();
        let kind = token.kind.to_string();
        writeln!(out, "{position:<8} {kind:<10} {}", file.slice(token.span))?;
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
        let file = SourceMap::new().add_file("test.xur", "let x =\n  \"hi\";");
        let (tokens, _) = Lexer::for_file(&file).tokenise();
        let mut out = Vec::new();
        write_token_table(&mut out, &file, &tokens).unwrap();
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
