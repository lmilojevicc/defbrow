use std::{
    io::{self, IsTerminal},
    process::ExitCode,
};

use anyhow::Result;
use clap::Parser;
use defbrow::{backend::system_backend, execute, ui, Cli};

fn run() -> Result<()> {
    let cli = Cli::parse();
    cli.check_terminal(io::stdin().is_terminal(), io::stdout().is_terminal())?;
    let backend = system_backend()?;
    execute(
        backend.as_ref(),
        cli.command.as_ref(),
        &mut io::stdout(),
        ui::pick,
    )
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("defbrow: {error:#}");
            ExitCode::FAILURE
        }
    }
}
