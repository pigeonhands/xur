mod debug;
mod run;

use anyhow::Result;

use crate::cli::Command;

pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Run { file } => run::run(&file),
        Command::Debug(command) => debug::run(command),
    }
}
