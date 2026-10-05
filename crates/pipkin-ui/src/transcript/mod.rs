//! Transcript: markdown, highlighting, document-level selection and the list view.

pub mod blocks;
pub mod document;
pub mod highlight;
pub mod markdown;
pub mod tools;
pub mod view;

pub use view::{TranscriptStats, TranscriptView, init};
