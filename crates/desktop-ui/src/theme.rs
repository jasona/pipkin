//! Semantic design tokens. Both variants are built from the same token set; views read tokens
//! through `cx.theme()` and never hard-code colors.

use desktop_core::{TextSize, Theme as ThemeChoice};
use gpui::{App, Global, Hsla, Pixels, SharedString, px, rgba};

#[derive(Clone, Debug)]
pub struct Colors {
    pub bg_app: Hsla,
    pub bg_pane: Hsla,
    pub bg_surface: Hsla,
    pub bg_elevated: Hsla,
    pub bg_input: Hsla,
    pub bg_hover: Hsla,
    pub bg_active: Hsla,
    pub bg_selected: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_faint: Hsla,
    pub accent: Hsla,
    pub accent_text: Hsla,
    pub accent_bg: Hsla,
    pub text_selection: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub danger_bg: Hsla,
    pub code_bg: Hsla,
    pub syn_keyword: Hsla,
    pub syn_string: Hsla,
    pub syn_comment: Hsla,
    pub syn_number: Hsla,
    pub syn_type: Hsla,
    pub syn_function: Hsla,
    pub diff_add_bg: Hsla,
    pub diff_remove_bg: Hsla,
    pub diff_add_text: Hsla,
    pub diff_remove_text: Hsla,
    pub diff_hunk_bg: Hsla,
    pub scrim: Hsla,
}

fn c(hex: u32) -> Hsla {
    rgba(hex).into()
}

impl Colors {
    pub fn dark() -> Self {
        Colors {
            bg_app: c(0x141518ff),
            bg_pane: c(0x191a1eff),
            bg_surface: c(0x1e1f24ff),
            bg_elevated: c(0x282a30ff),
            bg_input: c(0x1e1f24ff),
            bg_hover: c(0xffffff0d),
            bg_active: c(0xffffff16),
            bg_selected: c(0xffffff1c),
            border: c(0xffffff14),
            border_strong: c(0xffffff2a),
            text: c(0xe8e9ecff),
            text_muted: c(0xa3a6aeff),
            text_faint: c(0x73767fff),
            accent: c(0x8aa4ffff),
            accent_text: c(0x10131fff),
            accent_bg: c(0x8aa4ff24),
            text_selection: c(0x8aa4ff55),
            success: c(0x62c496ff),
            warning: c(0xe3b45eff),
            danger: c(0xf0747cff),
            danger_bg: c(0xf0747c1f),
            code_bg: c(0x16171bff),
            syn_keyword: c(0xc792eaff),
            syn_string: c(0x9ece8aff),
            syn_comment: c(0x6b7080ff),
            syn_number: c(0xf0a96aff),
            syn_type: c(0x7dcfe0ff),
            syn_function: c(0x8aa4ffff),
            diff_add_bg: c(0x62c49624),
            diff_remove_bg: c(0xf0747c24),
            diff_add_text: c(0x62c496ff),
            diff_remove_text: c(0xf0747cff),
            diff_hunk_bg: c(0x8aa4ff18),
            scrim: c(0x00000099),
        }
    }

    pub fn light() -> Self {
        Colors {
            bg_app: c(0xeeeeecff),
            bg_pane: c(0xf5f5f3ff),
            bg_surface: c(0xfdfdfcff),
            bg_elevated: c(0xffffffff),
            bg_input: c(0xffffffff),
            bg_hover: c(0x0000000b),
            bg_active: c(0x00000014),
            bg_selected: c(0x0000001a),
            border: c(0x00000016),
            border_strong: c(0x0000002e),
            text: c(0x1b1c20ff),
            text_muted: c(0x5a5d66ff),
            text_faint: c(0x80838cff),
            accent: c(0x3454d1ff),
            accent_text: c(0xffffffff),
            accent_bg: c(0x3454d118),
            text_selection: c(0x3454d140),
            success: c(0x1f8a5bff),
            warning: c(0x9a6a10ff),
            danger: c(0xc23a44ff),
            danger_bg: c(0xc23a4414),
            code_bg: c(0xf1f1efff),
            syn_keyword: c(0x8a3fb8ff),
            syn_string: c(0x2f7d32ff),
            syn_comment: c(0x8a8d96ff),
            syn_number: c(0xb25a0eff),
            syn_type: c(0x1a7a8cff),
            syn_function: c(0x3454d1ff),
            diff_add_bg: c(0x1f8a5b1c),
            diff_remove_bg: c(0xc23a441a),
            diff_add_text: c(0x1f7a52ff),
            diff_remove_text: c(0xc23a44ff),
            diff_hunk_bg: c(0x3454d112),
            scrim: c(0x00000055),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub choice: ThemeChoice,
    pub colors: Colors,
    pub scale: f32,
    pub reduced_motion: bool,
}

impl Global for Theme {}

pub const SPACE: f32 = 4.0;

impl Theme {
    pub fn new(choice: ThemeChoice, size: TextSize, reduced_motion: bool) -> Self {
        Theme {
            choice,
            colors: match choice {
                ThemeChoice::Dark => Colors::dark(),
                ThemeChoice::Light => Colors::light(),
            },
            scale: size.scale(),
            reduced_motion,
        }
    }

    /// Chrome text: menus, lists, labels.
    pub fn ui_size(&self) -> Pixels {
        px(14.0 * self.scale)
    }

    pub fn small_size(&self) -> Pixels {
        px(12.0 * self.scale)
    }

    /// Conversation prose.
    pub fn body_size(&self) -> Pixels {
        px(15.5 * self.scale)
    }

    pub fn body_line_height(&self) -> Pixels {
        px((15.5 * 1.55 * self.scale).round())
    }

    pub fn code_size(&self) -> Pixels {
        px(13.0 * self.scale)
    }

    pub fn code_line_height(&self) -> Pixels {
        px((13.0 * 1.55 * self.scale).round())
    }

    pub fn control_height(&self) -> Pixels {
        px((30.0 * self.scale.max(1.0)).round())
    }

    pub fn ui_font(&self) -> SharedString {
        crate::assets::UI_FONT.into()
    }

    pub fn mono_font(&self) -> SharedString {
        crate::assets::MONO_FONT.into()
    }
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}
