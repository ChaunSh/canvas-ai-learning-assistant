//! Tauri command surface.
//!
//! Every command that touches user data takes `tauri::WebviewWindow` and calls
//! `guard::require_main_window`. Application commands are not gated by Tauri
//! capabilities, so this label check is the authorization boundary that keeps
//! the remote Canvas workspace webview out of the local API entirely.

pub mod ai;
pub mod canvas;
pub mod canvas_files;
pub mod courses;
pub mod coursework;
pub mod data;
pub mod guard;
pub mod materials;
pub mod planning;
pub mod system;

pub use ai::*;
pub use canvas::*;
pub use canvas_files::*;
pub use courses::*;
pub use coursework::*;
pub use data::*;
pub use materials::*;
pub use planning::*;
pub use system::*;
