//! 水色ベースのクールなダークテーマ。
//!
//! iced の Theme::custom で使う Palette を定義する。
//! 薄い水色のアクセント + ダーク寄りの背景。

use iced::theme::Palette;
use iced::Color;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

/// アクセント色（水色）。status 表示など view 側からも直接使う。
pub const ACCENT: Color = Color {
    r: 0x5F as f32 / 255.0,
    g: 0xC6 as f32 / 255.0,
    b: 0xE8 as f32 / 255.0,
    a: 1.0,
};

/// 枠線色。
pub const BORDER: Color = Color {
    r: 0x2A as f32 / 255.0,
    g: 0x3B as f32 / 255.0,
    b: 0x4A as f32 / 255.0,
    a: 1.0,
};

/// 水色系ダークテーマの Palette。
pub fn aqua_dark() -> Palette {
    Palette {
        background: rgb(0x14, 0x1C, 0x24), // 暗めの青黒
        text: rgb(0xD8, 0xE6, 0xEE),       // 明るい水色がかった白
        primary: ACCENT,
        success: rgb(0x53, 0xC2, 0x8E),
        warning: rgb(0xE8, 0xB4, 0x5F),
        danger: rgb(0xE0, 0x60, 0x60),
    }
}
