//! Hand-made 5x7 mixed-case font (plus a descender row) for logs and the terminal.

use super::Px;
use eframe::egui::Color32;
use std::sync::OnceLock;

pub const CELL_W: i32 = 6;
pub const CELL_H: i32 = 9;

/// One hex pair per row, 5 bits wide (bit 4 = leftmost); an eighth pair is the descender.
const GLYPHS: &[(char, &str)] = &[
    ('A', "0E11111F111111"), ('B', "1E11111E11111E"), ('C', "0E11101010110E"), ('D', "1E11111111111E"),
    ('E', "1F10101E10101F"), ('F', "1F10101E101010"), ('G', "0E11101711110E"), ('H', "1111111F111111"),
    ('I', "0E04040404040E"), ('J', "0702020202120C"), ('K', "11121418141211"), ('L', "1010101010101F"),
    ('M', "111B1515111111"), ('N', "11191513111111"), ('O', "0E11111111110E"), ('P', "1E11111E101010"),
    ('Q', "0E11111115120D"), ('R', "1E11111E141211"), ('S', "0F10100E01011E"), ('T', "1F040404040404"),
    ('U', "1111111111110E"), ('V', "11111111110A04"), ('W', "11111115151B11"), ('X', "11110A040A1111"),
    ('Y', "11110A04040404"), ('Z', "1F01020408101F"),
    ('a', "00000E010F110F"), ('b', "10101E1111111E"), ('c', "00000F1010100F"), ('d', "01010F1111110F"),
    ('e', "00000E111F100E"), ('f', "06081E08080808"), ('g', "00000F11110F010E"), ('h', "10101E11111111"),
    ('i', "04000C0404040E"), ('j', "020006020202120C"), ('k', "10101214181412"), ('l', "0C04040404040E"),
    ('m', "00001A15151515"), ('n', "00001E11111111"), ('o', "00000E1111110E"), ('p', "00001E11111E1010"),
    ('q', "00000F11110F0101"), ('r', "00001618101010"), ('s', "00000F100E011E"), ('t', "08081E08080806"),
    ('u', "0000111111110F"), ('v', "00001111110A04"), ('w', "0000111115150A"), ('x', "0000110A040A11"),
    ('y', "00001111110F010E"), ('z', "00001F0204081F"),
    ('0', "0E11131519110E"), ('1', "040C040404040E"), ('2', "0E11010608101F"), ('3', "0E11010601110E"),
    ('4', "02060A121F0202"), ('5', "1F101E0101110E"), ('6', "0608101E11110E"), ('7', "1F010204080808"),
    ('8', "0E11110E11110E"), ('9', "0E11110F01020C"),
    ('!', "04040404040004"), ('"', "0A0A0A00000000"), ('#', "0A0A1F0A1F0A0A"), ('$', "040F140E051E04"),
    ('%', "18190204081303"), ('&', "0C12140815120D"), ('\'', "04040400000000"), ('(', "02040808080402"),
    (')', "08040202020408"), ('*', "0004150E150400"), ('+', "0004041F040400"), (',', "00000000000C0408"),
    ('-', "0000001F000000"), ('.', "00000000000C0C"), ('/', "01010204081010"), (':', "000C0C000C0C00"),
    (';', "000C0C000C0408"), ('<', "02040810080402"), ('=', "00001F001F0000"), ('>', "08040201020408"),
    ('?', "0E110102040004"), ('@', "0E11171517100E"), ('[', "0E08080808080E"), ('\\', "10100804020101"),
    (']', "0E02020202020E"), ('^', "040A1100000000"), ('_', "000000000000001F"), ('`', "08040200000000"),
    ('{', "02040408040402"), ('|', "0404040404040404"), ('}', "08040402040408"), ('~', "00000815020000"),
    ('•', "00000E0E0E0000"), ('…', "00000000000015"), ('→', "0004021F020400"), ('←', "0004081F080400"),
    ('✓', "00010102140800"), ('✔', "00010102140800"), ('✗', "00110A040A1100"),
    ('“', "0A0A0A00000000"), ('”', "0A0A0A00000000"), ('‘', "04040400000000"), ('’', "04040400000000"),
    ('–', "0000001F000000"), ('—', "0000001F000000"), ('❯', "08040201020408"), ('➜', "0004021F020400"),
];

/// Box-drawing lines as (left, right, up, down) arms.
fn arms(ch: char) -> Option<(bool, bool, bool, bool)> {
    Some(match ch {
        '─' | '━' | '═' => (true, true, false, false),
        '│' | '┃' | '║' => (false, false, true, true),
        '┌' | '╭' | '╔' | '┏' => (false, true, false, true),
        '┐' | '╮' | '╗' | '┓' => (true, false, false, true),
        '└' | '╰' | '╚' | '┗' => (false, true, true, false),
        '┘' | '╯' | '╝' | '┛' => (true, false, true, false),
        '├' | '╠' => (false, true, true, true),
        '┤' | '╣' => (true, false, true, true),
        '┬' | '╦' => (true, true, false, true),
        '┴' | '╩' => (true, true, true, false),
        '┼' | '╬' => (true, true, true, true),
        _ => return None,
    })
}

fn rows(ch: char) -> Option<[u8; 8]> {
    static T: OnceLock<std::collections::HashMap<char, [u8; 8]>> = OnceLock::new();
    let t = T.get_or_init(|| {
        let parse = |hex: &str| {
            let mut r = [0u8; 8];
            for (i, row) in r.iter_mut().enumerate().take(hex.len() / 2) {
                *row = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("font hex");
            }
            r
        };
        GLYPHS.iter().map(|(c, hex)| (*c, parse(hex))).collect()
    });
    t.get(&ch).copied()
}

/// Draws one character with its top-left cell corner at `x`,`y`.
pub fn draw_char(px: &Px, x: i32, y: i32, ch: char, c: Color32) {
    if ch.is_whitespace() || ch.is_control() {
        return;
    }
    if let Some((l, r, u, d)) = arms(ch) {
        let (x0, x1) = (if l { 0 } else { 2 }, if r { CELL_W } else { 3 });
        let (y0, y1) = (if u { 0 } else { 4 }, if d { CELL_H } else { 5 });
        if l || r {
            px.fill(x + x0, y + 4, x1 - x0, 1, c);
        }
        if u || d {
            px.fill(x + 2, y + y0, 1, y1 - y0, c);
        }
        return;
    }
    match ch {
        '█' => return px.fill(x, y, CELL_W, CELL_H, c),
        '▀' => return px.fill(x, y, CELL_W, 4, c),
        '▄' => return px.fill(x, y + 4, CELL_W, 5, c),
        '░' | '▒' | '▓' => return px.fill(x + 1, y + 1, CELL_W - 2, CELL_H - 2, c),
        _ => {}
    }
    let Some(glyph) = rows(ch).or_else(|| rows('?')) else { return };
    for (ry, bits) in glyph.iter().enumerate() {
        for col in 0..5 {
            if bits & (1 << (4 - col)) != 0 {
                px.fill(x + col, y + ry as i32, 1, 1, c);
            }
        }
    }
}

pub fn draw(px: &Px, x: i32, y: i32, s: &str, c: Color32) {
    for (i, ch) in s.chars().enumerate() {
        draw_char(px, x + i as i32 * CELL_W, y, ch, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_printable_ascii_has_a_distinct_glyph() {
        let mut seen = std::collections::HashSet::new();
        for ch in '!'..='~' {
            assert!(seen.insert(rows(ch).unwrap_or_else(|| panic!("missing {ch}"))), "duplicate {ch}");
        }
    }
}
