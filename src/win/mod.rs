//! Win32 backend: everything that talks to the operating system.
//!
//! Utility functions here are provided ahead of the modules that will use
//! them, so unused-code warnings are silenced inside this module tree.
#![allow(dead_code)]

pub mod api;
pub mod events;
pub mod monitor;
pub mod window;
