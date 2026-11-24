//! 7-segment LCD digits, 8x14 px, with unlit "ghost" segments.

use super::{pal, Px};
use eframe::egui::Color32;

pub const DIGIT_W: i32 = 8;
const GAP: i32 = 2;
const COLON_W: i32 = 4;

// a, b, c, d, e, f, g as (x, y, w, h)
const SEGS: [(i32, i32, i32, i32); 7] = [
    (1, 0, 6, 2),
    (6, 1, 2, 6),
    (6, 7, 2, 6),
    (1, 12, 6, 2),
    (0, 7, 2, 6),
    (0, 1, 2, 6),
    (1, 6, 6, 2),
];

fn mask(ch: char) -> u8 {
    // bit order: a b c d e f g
    match ch {
        '0' => 0b1111110,
        '1' => 0b0110000,
        '2' => 0b1101101,
        '3' => 0b1111001,
        '4' => 0b0110011,
        '5' => 0b1011011,
        '6' => 0b1011111,
        '7' => 0b1110000,
        '8' => 0b1111111,
        '9' => 0b1111011,
        '-' => 0b0000001,
        'd' => 0b0111101,
        _ => 0,
    }
}

pub fn width(s: &str) -> i32 {
    s.chars().map(|c| if c == ':' { COLON_W } else { DIGIT_W + GAP }).sum::<i32>() - GAP
}

/// Draws `s` (digits, ':', '-', 'd', ' '). `lit = false` draws only ghosts (blink).
pub fn draw(px: &Px, mut x: i32, y: i32, s: &str, lit: bool) {
    let on: Color32 = if lit { pal::LCD } else { pal::LCD_GHOST };
    for ch in s.chars() {
        if ch == ':' {
            px.fill(x + 1, y + 3, 2, 2, on);
            px.fill(x + 1, y + 9, 2, 2, on);
            x += COLON_W;
            continue;
        }
        let m = mask(ch);
        for (i, &(sx, sy, w, h)) in SEGS.iter().enumerate() {
            let set = m & (1 << (6 - i)) != 0;
            let c = if set && lit { pal::LCD } else { pal::LCD_GHOST };
            px.fill(x + sx, y + sy, w, h, c);
        }
        x += DIGIT_W + GAP;
    }
}
