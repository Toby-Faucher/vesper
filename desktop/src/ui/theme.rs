//! The web app's dark Vesper look (`web/src/styles/tokens.css`), as colours for GPUI and gpui-component.

use gpui::{App, Hsla, rgb, rgba};
use gpui_component::{Theme, ThemeMode};

pub const BG_APP: u32 = 0x100e0c;
pub const BG_SIDEBAR: u32 = 0x151311;
pub const BG_RAISED: u32 = 0x1a1816;
pub const BG_HOVER: u32 = 0x201e1b;
pub const BG_SELECTED: u32 = 0x262422;
pub const LINE_SUBTLE: u32 = 0x2d2b29;
pub const LINE: u32 = 0x3a3835;
pub const FG: u32 = 0xeeecea;
pub const FG_2: u32 = 0xb3b1ad;
pub const FG_3: u32 = 0x888683;
pub const ACCENT: u32 = 0xf0b010;
pub const ACCENT_HOVER: u32 = 0xffbe15;
pub const ACCENT_FG: u32 = 0xf7c243;
pub const ACCENT_SUBTLE: u32 = 0x2f1e00;
pub const ON_ACCENT: u32 = 0x100e0c;
pub const DANGER_FG: u32 = 0xf66d67;
pub const DANGER_SOLID: u32 = 0xba2b2e;
pub const WARN_FG: u32 = 0xf7945a;

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Dark mode, with gpui-component's colours set from the tokens.
pub fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let t = Theme::global_mut(cx);
    t.background = c(BG_APP);
    t.foreground = c(FG);
    t.border = c(LINE);
    t.input = c(LINE);
    t.ring = c(ACCENT);
    t.caret = c(ACCENT);
    t.selection = rgba(0xf0b01055).into();
    t.primary = c(ACCENT);
    t.primary_hover = c(ACCENT_HOVER);
    t.primary_active = c(ACCENT_FG);
    t.primary_foreground = c(ON_ACCENT);
    t.secondary = c(BG_RAISED);
    t.secondary_hover = c(BG_HOVER);
    t.secondary_active = c(BG_SELECTED);
    t.secondary_foreground = c(FG);
    t.muted = c(BG_RAISED);
    t.muted_foreground = c(FG_3);
    t.accent = c(BG_HOVER);
    t.accent_foreground = c(FG);
    t.danger = c(DANGER_SOLID);
    t.danger_foreground = c(FG);
    t.popover = c(BG_RAISED);
    t.popover_foreground = c(FG);
    t.sidebar = c(BG_SIDEBAR);
    t.sidebar_foreground = c(FG_2);
    t.sidebar_border = c(LINE_SUBTLE);
    t.list = c(BG_APP);
    t.list_hover = c(BG_HOVER);
    t.list_active = c(BG_SELECTED);
    t.scrollbar = c(BG_APP);
    t.scrollbar_thumb = c(LINE);
    t.scrollbar_thumb_hover = c(FG_3);
}
