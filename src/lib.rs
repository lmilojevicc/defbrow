pub mod backend;
pub mod ui;

use std::{io::Write, time::Duration};

use anyhow::{bail, Context, Result};
use backend::{Backend, Browser, Cancelled, CurrentDefaults};
use clap::{Parser, Subcommand};
use crossterm::event::{self, Event};

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

// The session keeps its terminal for the whole picker run, so the switch callback
// is invoked with the picker screen and raw mode still active.
pub fn execute(
    backend: &dyn Backend,
    command: Option<&Command>,
    output: &mut dyn Write,
    picker: impl FnOnce(ui::Picker, &mut dyn FnMut(&mut ui::Picker, &Browser)) -> Result<()>,
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
            picker(
                ui::Picker::new(browsers, current),
                &mut |picker, browser| {
                    switch_interactively(backend, picker, browser);
                },
            )?;
        }
    }
    Ok(())
}

fn switch_interactively(backend: &dyn Backend, picker: &mut ui::Picker, browser: &Browser) {
    let set_result = backend.set_default(browser, &cancel_requested);
    // Read actual state even after rejection: native setters can make partial changes.
    let current_result = backend.current();
    let browsers_result = backend.browsers();
    let cancelled = set_result
        .as_ref()
        .err()
        .and_then(|error| error.downcast_ref::<Cancelled>());
    let mut status = if let Some(cancelled) = cancelled {
        // A cancelled attempt reports the readback: the pending OS request may still apply.
        let stopped = if cancelled.requested {
            "Stopped waiting for approval."
        } else {
            // Nothing reached the OS, so no wait or prompt happened.
            "Stopped before requesting a change."
        };
        match &current_result {
            Ok(current) => format!(
                "{stopped} HTTP: {}; HTTPS: {}",
                current.http.as_deref().unwrap_or("(none)"),
                current.https.as_deref().unwrap_or("(none)")
            ),
            Err(_) => stopped.to_owned(),
        }
    } else {
        match &set_result {
            Err(error) => format!("Could not set {}: {error:#}", browser.name),
            Ok(()) => match &current_result {
                Ok(current)
                    if current.http.as_deref() == Some(&browser.id)
                        && current.https.as_deref() == Some(&browser.id) =>
                {
                    format!("Default browser: {}", browser.name)
                }
                Ok(_) => format!(
                    "Change to {} not verified; defaults differ. A partial change may have occurred.",
                    browser.name
                ),
                Err(_) => format!("Change to {} requested, but not verified.", browser.name),
            },
        }
    };
    if let Err(error) = &current_result {
        status.push_str(&format!(
            " Reading defaults failed (current unknown): {error:#}"
        ));
    }
    if let Err(error) = &browsers_result {
        status.push_str(&format!(
            " Browser refresh failed (previous entries shown): {error:#}"
        ));
    }
    // Unknown readback clears stale markers instead of presenting an old default as fact.
    picker.refresh(browsers_result.ok(), current_result.unwrap_or_default());
    picker.set_status(status);
}

/// Reports whether Esc or Ctrl-C was pressed while a setter is waiting for OS
/// approval. Only cancel keys are consumed; any other key pressed during the wait
/// is discarded, and poll/read errors leave the wait running.
fn cancel_requested() -> bool {
    if !matches!(event::poll(Duration::ZERO), Ok(true)) {
        return false;
    }
    match event::read() {
        Ok(Event::Key(key)) => ui::is_cancel_key(&key),
        _ => false,
    }
}

fn set_browser(backend: &dyn Backend, browser: &Browser, output: &mut dyn Write) -> Result<()> {
    writeln!(
        output,
        "Setting default browser to {}. OS confirmation may be required...",
        display_text(&browser.name)
    )?;
    output.flush()?;
    // No picker input to observe here: this path has no interactive wait to stop.
    backend.set_default(browser, &|| false)?;
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
