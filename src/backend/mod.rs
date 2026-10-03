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

pub trait Backend {
    fn browsers(&self) -> Result<Vec<Browser>>;
    fn current(&self) -> Result<CurrentDefaults>;
    /// A partial change is an error, with the actual per-scheme state reported.
    fn set_default(&self, browser: &Browser) -> Result<()>;
}

pub fn system_backend() -> Result<Box<dyn Backend>> {
    #[cfg(target_os = "linux")]
    return Ok(Box::new(linux::SystemBackend::new()?));

    #[cfg(target_os = "macos")]
    return Ok(Box::new(macos::SystemBackend::new()?));

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    anyhow::bail!("defbrow supports macOS and Linux only")
}
