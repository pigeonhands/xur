use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// The Xur programming language.
#[derive(Debug, Parser)]
#[command(name = "xur", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Inspect the output of individual compiler stages.
    #[command(subcommand)]
    Debug(DebugCommand),
}

#[derive(Debug, Subcommand)]
pub enum DebugCommand {
    /// Print the tokens the lexer produces for a source file.
    Tokens {
        /// The `.xur` file to lex.
        file: PathBuf,
    },
    /// Print the syntax tree the parser produces for a source file.
    Ast {
        /// The `.xur` file to parse.
        file: PathBuf,
    },
}
