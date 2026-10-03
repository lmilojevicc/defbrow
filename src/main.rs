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

fn diagnostic_text(error: &anyhow::Error) -> String {
    // Sanitize the complete chain: backend errors can contain untrusted paths,
    // handler IDs, native messages, or subprocess stderr.
    format!("{error:#}")
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("defbrow: {}", diagnostic_text(&error));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::diagnostic_text;

    #[test]
    fn diagnostic_preserves_error_chain_without_control_characters() {
        let error = anyhow::anyhow!("native\r\n\t\u{9b} failure HTTP=old, HTTPS=new")
            .context("path\u{1b}[31m/name\0 — Über");
        let text = diagnostic_text(&error);
        assert_eq!(
            text,
            "path [31m/name  — Über: native     failure HTTP=old, HTTPS=new"
        );
        assert!(text.chars().all(|c| !c.is_control()));
    }
}
