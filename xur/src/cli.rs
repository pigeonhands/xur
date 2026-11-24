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
    /// Run a program's `main` function.
    #[command(trailing_var_arg = true)]
    Run {
        /// The root `.xur` file, a directory (runs `main.xur` inside it),
        /// or `-` for stdin. Defaults to `main.xur` in the current directory.
        file: Option<PathBuf>,
        /// Don't show the `?` locations an error passed through when it
        /// reaches `main`.
        #[arg(long)]
        no_trace: bool,
        /// Arguments passed to the program as `std::argv()`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Start the language server (LSP), communicating over stdio.
    Lsp,
    /// Inspect the output of individual compiler stages.
    #[command(subcommand)]
    Debug(DebugCommand),
    /// Inject the xur-stub into a running process (Linux x86-64).
    #[command(subcommand)]
    Stub(StubCommand),
}

#[derive(Debug, Subcommand)]
pub enum StubCommand {
    /// Inject `libxur_stub.so` into a running process and run a `.xur` script
    /// inside it — the attach-time equivalent of `LD_PRELOAD` + `XUR_STUB_PATH`.
    Inject {
        /// The `.xur` file (or a directory of `*.xur`) to run in the target.
        script: PathBuf,
        /// Target process by PID.
        #[arg(long, conflicts_with = "name")]
        pid: Option<u32>,
        /// Target process by executable name.
        #[arg(long, conflicts_with = "pid")]
        name: Option<String>,
        /// Override the `libxur_stub.so` path instead of auto-resolving it.
        #[arg(long)]
        stub: Option<PathBuf>,
        /// Map the stub manually instead of via the OS loader (`dlopen`).
        /// Resolves imports against the target's loaded libraries and does not
        /// register the module with the dynamic linker.
        #[arg(long)]
        manual: bool,
    },
    /// Print the resolved `libxur_stub.so` path.
    Path,
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
    /// Print what every name in a program resolves to, following imports.
    Resolve {
        /// The root `.xur` file of the program.
        file: PathBuf,
    },
    /// Print the bytecode of every function in a source file.
    Bytecode {
        /// The root `.xur` file of the program.
        file: PathBuf,
    },
}
