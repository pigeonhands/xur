mod debug;
mod lsp;
mod run;

use std::io::Read;
use std::path::PathBuf;

use anyhow::Result;

use crate::cli::Command;

pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Run {
            file,
            no_trace,
            args,
        } => {
            let tracing = !no_trace;
            match file.as_deref().and_then(|p| p.to_str()) {
                Some("-") => {
                    let mut src = String::new();
                    std::io::stdin().read_to_string(&mut src)?;
                    xur_std::set_argv(None, args);
                    run::run_source("<stdin>".into(), src, tracing)
                }
                _ => {
                    let path = match file {
                        Some(p) if p.is_dir() => p.join("main.xur"),
                        Some(p) => p,
                        None => PathBuf::from("main.xur"),
                    };
                    let script = path.display().to_string();
                    xur_std::set_argv(Some(script), args);
                    run::run(&path, tracing)
                }
            }
        }
        Command::Lsp => lsp::run(),
        Command::Debug(command) => debug::run(command),
    }
}
