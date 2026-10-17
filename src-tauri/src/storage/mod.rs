//! Persistence layer: one SQLite file, versioned migrations, typed repositories.
//!
//! Concurrency model: a single `Connection` behind a `parking_lot::Mutex`.
//! SQLite serialises writers anyway, WAL gives concurrent readers, and every
//! repository function is scoped to a short statement batch so long jobs
//! (parsing, AI streaming) never hold the lock.

pub mod ai_repo;
pub mod canvas_file_repo;
pub mod course_repo;
pub mod coursework_repo;
pub mod db;
pub mod document_repo;
pub mod migrations;
pub mod planning_repo;
pub mod settings;
pub mod sync_repo;

pub use db::Db;
