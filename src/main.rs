//! yumi-wini: a scrollable-tiling window manager for Windows, inspired by Niri.
//!
//! This program acts as a window *management layer* on top of the existing
//! Windows window manager / DWM. It tracks application windows (HWNDs),
//! arranges them into a scrollable column-based layout, and drives focus
//! and animation — without replacing the OS compositor.

mod app;
mod logging;

use std::process::ExitCode;

fn main() -> ExitCode {
    logging::init();

    log::info!("yumi-wini starting (pid {})", std::process::id());

    match app::App::new() {
        Ok(mut app) => {
            if let Err(err) = app.run() {
                log::error!("fatal error: {err}");
                ExitCode::FAILURE
            } else {
                log::info!("yumi-wini exited cleanly");
                ExitCode::SUCCESS
            }
        }
        Err(err) => {
            log::error!("failed to start: {err}");
            ExitCode::FAILURE
        }
    }
}
