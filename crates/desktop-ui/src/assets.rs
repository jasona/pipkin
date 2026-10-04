//! Compile-time bundled fonts and icons. Provenance is recorded in `assets/PROVENANCE.md`.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        const ICONS: &[(&str, &[u8])] = &[
            $((concat!("icons/", $name, ".svg"),
               include_bytes!(concat!("../../../assets/icons/", $name, ".svg")) as &[u8])),*
        ];
    };
}

icons!(
    "plus",
    "search",
    "folder",
    "message-square",
    "chevron-right",
    "chevron-down",
    "chevron-up",
    "x",
    "paperclip",
    "arrow-up",
    "square",
    "settings",
    "file-text",
    "file-diff",
    "check",
    "triangle-alert",
    "circle-alert",
    "loader-circle",
    "panel-left",
    "panel-right",
    "terminal",
    "pencil",
    "trash-2",
    "copy",
    "arrow-down",
    "command",
    "sun",
    "moon",
    "wrench",
    "clock",
    "refresh-cw",
    "file-plus",
    "folder-open",
    "ellipsis",
    "circle-help",
    "circle-check",
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(p, _)| p.starts_with(path))
            .map(|(p, _)| SharedString::from(*p))
            .collect())
    }
}

macro_rules! font {
    ($file:literal) => {
        Cow::Borrowed(include_bytes!(concat!("../../../assets/fonts/", $file)) as &[u8])
    };
}

pub const UI_FONT: &str = "IBM Plex Sans";
pub const MONO_FONT: &str = "Lilex";

pub fn load_fonts(cx: &gpui::App) -> Result<()> {
    cx.text_system().add_fonts(vec![
        font!("IBMPlexSans-Regular.ttf"),
        font!("IBMPlexSans-Italic.ttf"),
        font!("IBMPlexSans-SemiBold.ttf"),
        font!("IBMPlexSans-SemiBoldItalic.ttf"),
        font!("Lilex-Regular.ttf"),
        font!("Lilex-Bold.ttf"),
        font!("Lilex-Italic.ttf"),
        font!("Lilex-BoldItalic.ttf"),
    ])
}
