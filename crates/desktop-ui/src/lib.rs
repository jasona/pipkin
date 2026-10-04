//! GPUI views, shared controls, theme and text layer.

pub mod assets;
pub mod model;
pub mod shell;
pub mod text;
pub mod theme;
pub mod transcript;

pub use model::{DemoControls, EffectHandler, Model};
pub use theme::{ActiveTheme, Theme};
