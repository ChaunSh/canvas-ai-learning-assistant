//! Document parsing, chunking and indexing.

pub mod archive;
pub mod chunker;
pub mod docx;
pub mod extract;
pub mod ics;
pub mod import;
pub mod pdf;
pub mod pipeline;
pub mod plain;
pub mod pptx;
pub mod tokenize;
pub mod xmlutil;

pub use chunker::{Chunk, ChunkOptions, ExtractedPage};
pub use extract::{detect_kind, Extraction};
