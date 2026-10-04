//! Backend adapters. Only the deterministic demo adapter exists today; a Pi adapter will
//! implement `desktop_core::Backend` beside it.

pub mod demo;
mod diff;
mod fixtures;
mod rng;
pub mod script;
mod story;

#[cfg(test)]
mod tests;

pub use demo::{DemoBackend, DemoOptions};
