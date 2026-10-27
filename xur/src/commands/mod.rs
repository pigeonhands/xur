mod debug;
mod run;

use anyhow::Result;

use crate::cli::Command;

pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Run { file, no_trace } => run::run(&file, !no_trace),
        Command::Debug(command) => debug::run(command),
    }
}
