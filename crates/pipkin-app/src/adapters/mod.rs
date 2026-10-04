//! Backend adapters: the Pi engine adapter (real mode) and the deterministic demo adapter
//! (explicit `--demo`, kept for regression tests).

pub mod demo;
mod diff;
mod fixtures;
pub mod pi;
mod rng;
pub mod script;
mod story;

#[cfg(test)]
mod tests;

pub use demo::{DemoBackend, DemoOptions};
