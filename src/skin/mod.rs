//! Procedural pixel skin: palette, a scaled pixel painter, bevels and icons.
//! Everything is drawn in "skin pixels" and multiplied by an integer scale.

pub mod font;
pub mod lcd;

use std::cell::RefCell;

use eframe::egui::{Color32, Pos2, Rect, Vec2};

pub mod pal {
    use eframe::egui::Color32;
    const fn c(r: u8, g: u8, b: u8) -> Color32 {
        Color32::from_rgb(r, g, b)
    }
    pub const CHROME: Color32 = c(0x38, 0x38, 0x4c);
    pub const CHROME_HI: Color32 = c(0x6a, 0x6a, 0x86);
    pub const CHROME_LO: Color32 = c(0x16, 0x16, 0x22);
    pub const CHROME_TEXT: Color32 = c(0xc8, 0xc8, 0xdc);
    pub const TITLE_A: Color32 = c(0x9c, 0x94, 0x6c);
    pub const TITLE_B: Color32 = c(0x4e, 0x4a, 0x38);
    pub const TITLE_TEXT: Color32 = c(0xe8, 0xe8, 0xf0);
    pub const BUTTON: Color32 = c(0x4c, 0x4c, 0x64);
    pub const ICON: Color32 = c(0xd4, 0xd4, 0xe4);
    pub const LCD_BG: Color32 = c(0x00, 0x00, 0x00);
    pub const LCD: Color32 = c(0x00, 0xe8, 0x00);
    pub const LCD_GHOST: Color32 = c(0x0c, 0x2c, 0x0c);
    pub const LCD_DIM: Color32 = c(0x1e, 0x78, 0x1e);
    pub const YELLOW: Color32 = c(0xe8, 0xe0, 0x20);
    pub const RED: Color32 = c(0xff, 0x30, 0x20);
    pub const SELECT_BG: Color32 = c(0x00, 0x00, 0xc6);
    pub const WHITE: Color32 = c(0xff, 0xff, 0xff);
    pub const HEADER: Color32 = c(0xa8, 0xb4, 0xff);
}

/// Native-resolution framebuffer the whole skin is drawn into each frame,
/// then uploaded as one texture and scaled up with nearest-neighbour filtering.
pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub pixels: Vec<Color32>,
}

impl Canvas {
    pub fn new(w: i32, h: i32) -> Self {
        Self { w, h, pixels: vec![Color32::BLACK; (w * h) as usize] }
    }
}

/// Painter that works in integer skin pixels. `oy` offsets into the canvas (panel stacking);
/// `origin`/`s` map skin pixels to screen points for hit-testing.
pub struct Px<'a> {
    pub canvas: &'a RefCell<Canvas>,
    pub oy: i32,
    pub origin: Pos2,
    pub s: f32,
}

impl<'a> Px<'a> {
    pub fn at(&self, dy: i32) -> Px<'a> {
        Px { canvas: self.canvas, oy: self.oy + dy, origin: self.origin, s: self.s }
    }

    pub fn rect_of(&self, x: i32, y: i32, w: i32, h: i32) -> Rect {
        let min = self.origin + Vec2::new(x as f32, (y + self.oy) as f32) * self.s;
        Rect::from_min_size(min, Vec2::new(w as f32, h as f32) * self.s)
    }

    pub fn fill(&self, x: i32, y: i32, w: i32, h: i32, c: Color32) {
        let mut cv = self.canvas.borrow_mut();
        let y = y + self.oy;
        let (x0, y0) = (x.max(0), y.max(0));
        let (x1, y1) = ((x + w).min(cv.w), (y + h).min(cv.h));
        let cw = cv.w;
        for yy in y0..y1 {
            let row = (yy * cw) as usize;
            cv.pixels[row + x0 as usize..row + x1.max(x0) as usize].fill(c);
        }
    }

    /// Raised (or sunken) 1px bevelled box.
    pub fn bevel(&self, x: i32, y: i32, w: i32, h: i32, face: Color32, raised: bool) {
        let (tl, br) = if raised { (pal::CHROME_HI, pal::CHROME_LO) } else { (pal::CHROME_LO, pal::CHROME_HI) };
        self.fill(x, y, w, h, face);
        self.fill(x, y, w, 1, tl);
        self.fill(x, y, 1, h, tl);
        self.fill(x, y + h - 1, w, 1, br);
        self.fill(x + w - 1, y, 1, h, br);
    }

    /// Black inset display well.
    pub fn well(&self, x: i32, y: i32, w: i32, h: i32) {
        self.bevel(x - 1, y - 1, w + 2, h + 2, pal::LCD_BG, false);
    }

    /// Panel body + striped title bar with centred caption.
    pub fn panel(&self, w: i32, h: i32, title: &str) {
        self.bevel(0, 0, w, h, pal::CHROME, true);
        let tw = font::width(title);
        let tx = (w - tw) / 2;
        for i in 0..3 {
            let y = 4 + i * 2;
            self.fill(6, y, tx - 10, 1, pal::TITLE_A);
            self.fill(6, y + 1, tx - 10, 1, pal::TITLE_B);
            self.fill(tx + tw + 4, y, w - (tx + tw + 4) - 42, 1, pal::TITLE_A);
            self.fill(tx + tw + 4, y + 1, w - (tx + tw + 4) - 42, 1, pal::TITLE_B);
        }
        self.text(tx, 4, title, pal::TITLE_TEXT);
    }

    pub fn text(&self, x: i32, y: i32, s: &str, c: Color32) {
        font::draw(self, x, y, s, c);
    }

    /// Draw a bitmap given as rows of `#`/`.`.
    pub fn bitmap(&self, x: i32, y: i32, rows: &[&str], c: Color32) {
        for (ry, row) in rows.iter().enumerate() {
            let b = row.as_bytes();
            let mut cx = 0;
            while cx < b.len() {
                if b[cx] == b'#' {
                    let start = cx;
                    while cx < b.len() && b[cx] == b'#' {
                        cx += 1;
                    }
                    self.fill(x + start as i32, y + ry as i32, (cx - start) as i32, 1, c);
                } else {
                    cx += 1;
                }
            }
        }
    }
}

pub mod icon {
    pub const PREV: &[&str] = &["##...#", "##..##", "##.###", "######", "##.###", "##..##", "##...#"];
    pub const PLAY: &[&str] = &["#....", "##...", "###..", "####.", "###..", "##...", "#...."];
    pub const PAUSE: &[&str] = &["##.##", "##.##", "##.##", "##.##", "##.##", "##.##", "##.##"];
    pub const STOP: &[&str] = &["######", "######", "######", "######", "######", "######"];
    pub const NEXT: &[&str] = &["#...##", "##..##", "###.##", "######", "###.##", "##..##", "#...##"];
    pub const RESTART: &[&str] = &[".####.#", "#....##", "#...###", "#......", "#.....#", ".#...#.", "..###.."];
    pub const STATE_PLAY: &[&str] = &["#....", "##...", "###..", "####.", "###..", "##...", "#...."];
    pub const STATE_PAUSE: &[&str] = &["##.##", "##.##", "##.##", "##.##", "##.##", "##.##", "##.##"];
    pub const STATE_STOP: &[&str] = &[".....", "#####", "#####", "#####", "#####", "#####", "....."];
    pub const CLOSE: &[&str] = &["#...#", ".#.#.", "..#..", ".#.#.", "#...#"];
    pub const MIN: &[&str] = &[".....", ".....", ".....", ".....", "#####"];
}
