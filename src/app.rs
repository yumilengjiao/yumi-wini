//! Top-level application shell.
//!
//! `App` owns the message loop. Concrete subsystems (window tracking,
//! layout engine, input handling, …) will be attached here in later
//! modules.

use std::fmt;

/// Fatal, top-level error.
#[derive(Debug)]
pub struct AppError(String);

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AppError {}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError(s)
    }
}

pub struct App;

impl App {
    /// Construct the application. Subsystems will be initialized here
    /// as they are implemented.
    pub fn new() -> Result<Self, AppError> {
        Ok(App)
    }

    /// Run the main loop. Currently a stub; will become the Win32
    /// message pump that drives event hooks, input and animations.
    pub fn run(&mut self) -> Result<(), AppError> {
        log::info!("nothing to do yet — stub run loop");
        Ok(())
    }
}
