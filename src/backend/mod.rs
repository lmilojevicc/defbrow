use std::fmt;

use anyhow::Result;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Browser {
    pub id: String,
    pub name: String,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CurrentDefaults {
    pub http: Option<String>,
    pub https: Option<String>,
}

/// A wait for OS approval that the caller stopped before it completed.
///
/// The underlying OS request cannot be undone and may still apply later, so a
/// cancelled attempt reports the observed state instead of a result.
#[derive(Debug)]
pub struct Cancelled {
    /// Whether a request reached the OS before waiting was stopped. `false` means
    /// nothing was requested, so no prompt was shown and no change can be pending.
    pub requested: bool,
}

impl fmt::Display for Cancelled {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("stopped waiting for OS approval")
    }
}

impl std::error::Error for Cancelled {}

pub trait Backend {
    fn browsers(&self) -> Result<Vec<Browser>>;
    fn current(&self) -> Result<CurrentDefaults>;
    /// A partial change is an error, with the actual per-scheme state reported.
    ///
    /// `interrupted` is polled while waiting for OS approval, at intervals of at
    /// most 50ms. When it reports true, waiting stops with [`Cancelled`]: no
    /// further request is issued and success is never claimed.
    fn set_default(&self, browser: &Browser, interrupted: &dyn Fn() -> bool) -> Result<()>;
}

pub fn system_backend() -> Result<Box<dyn Backend>> {
    #[cfg(target_os = "linux")]
    return Ok(Box::new(linux::SystemBackend::new()?));

    #[cfg(target_os = "macos")]
    return Ok(Box::new(macos::SystemBackend::new()?));

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    anyhow::bail!("defbrow supports macOS and Linux only")
}
