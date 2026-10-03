pub mod backend;
pub mod ui;

use std::io::Write;

use anyhow::{bail, Context, Result};
use backend::{Backend, Browser, CurrentDefaults};
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List registered HTTP and HTTPS handlers and their IDs
    List,
    /// Show the current handler for each URL scheme
    Current,
    /// Set the default browser by exact ID or unambiguous name
    Set { browser: String },
}

impl Cli {
    pub fn check_terminal(&self, stdin_is_terminal: bool, stdout_is_terminal: bool) -> Result<()> {
        if self.command.is_none() && !(stdin_is_terminal && stdout_is_terminal) {
            bail!("The interactive picker needs a terminal on stdin and stdout. Use `defbrow list`, `defbrow current`, or `defbrow set <id>` instead.");
        }
        Ok(())
    }
}

pub fn resolve_browser<'a>(browsers: &'a [Browser], query: &str) -> Result<&'a Browser> {
    if let Some(browser) = browsers.iter().find(|browser| browser.id == query) {
        return Ok(browser);
    }
    let query = query.to_lowercase();
    let mut matches = browsers
        .iter()
        .filter(|browser| browser.name.to_lowercase() == query);
    let Some(browser) = matches.next() else {
        bail!("No registered browser matches that ID or name. Use `defbrow list` to see IDs.");
    };
    if matches.next().is_some() {
        bail!("That browser name is ambiguous. Use an exact ID from `defbrow list`.");
    }
    Ok(browser)
}

// Picker owns only selection; setting happens after it returns and restores the terminal.
pub fn execute(
    backend: &dyn Backend,
    command: Option<&Command>,
    output: &mut dyn Write,
    picker: impl FnOnce(Vec<Browser>, CurrentDefaults) -> Result<Option<Browser>>,
) -> Result<()> {
    match command {
        Some(Command::List) => {
            let browsers = backend.browsers()?;
            let current = backend.current()?;
            if browsers.is_empty() {
                writeln!(output, "No registered HTTP+HTTPS handlers found.")?;
            }
            for browser in browsers {
                writeln!(
                    output,
                    "{}\t{}\t{}\t{}",
                    display_text(&browser.name),
                    display_text(&browser.id),
                    ui::current_marker(&browser, &current),
                    display_text(&browser.detail)
                )?;
            }
        }
        Some(Command::Current) => print_current(output, &backend.current()?)?,
        Some(Command::Set { browser }) => {
            let browsers = backend.browsers()?;
            let browser = resolve_browser(&browsers, browser)?;
            set_browser(backend, browser, output)?;
        }
        None => {
            let browsers = backend.browsers()?;
            let current = backend.current()?;
            if let Some(browser) = picker(browsers, current)? {
                set_browser(backend, &browser, output)?;
            }
        }
    }
    Ok(())
}

fn set_browser(backend: &dyn Backend, browser: &Browser, output: &mut dyn Write) -> Result<()> {
    writeln!(
        output,
        "Setting HTTP and HTTPS to {}. OS confirmation may be required...",
        display_text(&browser.name)
    )?;
    output.flush()?;
    backend.set_default(browser)?;
    let current = backend
        .current()
        .context("Change requested, but reading defaults for verification failed")?;
    if current.http.as_deref() != Some(&browser.id) || current.https.as_deref() != Some(&browser.id)
    {
        bail!("Default change was not verified: requested {}; HTTP={}, HTTPS={}. A partial change may have occurred.", display_text(&browser.id), display_text(current.http.as_deref().unwrap_or("(none)")), display_text(current.https.as_deref().unwrap_or("(none)")));
    }
    writeln!(output, "Default browser: {}", display_text(&browser.name))?;
    Ok(())
}

fn print_current(output: &mut dyn Write, current: &CurrentDefaults) -> Result<()> {
    writeln!(
        output,
        "HTTP: {}",
        display_text(current.http.as_deref().unwrap_or("(none)"))
    )?;
    writeln!(
        output,
        "HTTPS: {}",
        display_text(current.https.as_deref().unwrap_or("(none)"))
    )?;
    Ok(())
}

pub(crate) fn display_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
