//! Domain models shared by the Rust services, the SQLite layer and the
//! TypeScript frontend. Every serialised struct uses camelCase on the wire.
//!
//! Design rule: Canvas identifiers are always `String`. Canvas ids are 64-bit
//! integers; JSON numbers would lose precision in JavaScript, so we never
//! coerce them.

pub mod ai;
pub mod canvas;
pub mod course;
pub mod material;
pub mod planning;

pub use ai::*;
pub use canvas::*;
pub use course::*;
pub use material::*;
pub use planning::*;

use serde::{Deserialize, Deserializer, Serialize};

/// Deserialiser for tri-state patch fields.
///
/// With `#[serde(default, deserialize_with = "double_option")]` the three
/// states become distinguishable:
///   * field absent  -> `None`         (leave unchanged)
///   * field `null`  -> `Some(None)`   (clear the value)
///   * field value   -> `Some(Some(v))`(set the value)
pub fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Where a record came from. Kept separate from `sync_status` so the UI can
/// always distinguish Canvas-sourced data from user-created or AI-generated
/// data (task specification §43.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Canvas,
    Manual,
    Import,
    Ai,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Canvas => "canvas",
            SourceKind::Manual => "manual",
            SourceKind::Import => "import",
            SourceKind::Ai => "ai",
        }
    }

    pub fn from_str(value: &str) -> SourceKind {
        match value {
            "canvas" => SourceKind::Canvas,
            "ai" => SourceKind::Ai,
            "import" => SourceKind::Import,
            _ => SourceKind::Manual,
        }
    }
}

/// Generic paging envelope used by list commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub offset: u32,
    pub limit: u32,
}
