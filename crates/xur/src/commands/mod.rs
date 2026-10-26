mod debug;

use anyhow::Result;

use crate::cli::Command;

pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Debug(command) => debug::run(command),
    }
}
