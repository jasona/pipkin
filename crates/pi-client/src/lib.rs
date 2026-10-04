//! Rust client for Pi's experimental service protocol (protocol version 8) and the Chord
//! service layer carried inside it. No GPUI and no application types: this crate speaks the
//! wire; the desktop adapter maps it onto Pipkin's core.

pub mod cbor;
pub mod chord;
pub mod client;
pub mod delta;
pub mod error;
pub mod frame;
pub mod protocol;
pub mod testing;
pub mod unix;

pub use error::{Error, Result};
