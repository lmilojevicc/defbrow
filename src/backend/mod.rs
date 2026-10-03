use anyhow::{bail, Result};

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
    bail!("The system browser backend is not implemented yet")
}
