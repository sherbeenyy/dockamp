//! Console panel to the right of the player: LOGS and TERMINAL tabs for the selection.

use eframe::egui::{self, Color32, Event, Key, Modifiers, Ui};

use super::{button, hit};
use crate::console::{lock, COLS, ROWS};
use crate::model::fit;
use crate::skin::{font, icon, mono, pal, Px};
use crate::App;

pub const CONSOLE_W: i32 = COLS as i32 * mono::CELL_W + 20;
const WELL_X: i32 = 8;
const WELL_Y: i32 = 29;
const WELL_W: i32 = CONSOLE_W - 16;
const WELL_H: i32 = ROWS as i32 * mono::CELL_H + 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Logs,
    Terminal,
}

pub fn draw(app: &mut App, ui: &Ui, px: &Px) {
    px.panel(CONSOLE_W, super::H, "DOCKAMP CONSOLE");
    super::title_drag_w(ui, px, "title-console", CONSOLE_W);
    if button(ui, px, "console-close", CONSOLE_W - 14, 3, 9, 9, icon::CLOSE) {
        app.close_console(ui.ctx());
        return;
    }

    for (tab, label, x) in [(Tab::Logs, "LOGS", 8), (Tab::Terminal, "TERMINAL", 66)] {
        let active = app.tab == tab;
        px.bevel(x, 16, 56, 12, if active { pal::BUTTON } else { pal::CHROME }, !active);
        let col = if active { pal::LCD } else { pal::CHROME_TEXT };
        px.text(x + (56 - font::width(label)) / 2, 20, label, col);
        if hit(ui, px, label, x, 16, 56, 12).clicked() {
            app.open_console(ui.ctx(), tab);
        }
    }
    let name = app.selected_info().map_or(String::new(), |c| fit(&c.name, 40));
    px.text(CONSOLE_W - 10 - font::width(&name), 20, &name, pal::HEADER);

    px.well(WELL_X, WELL_Y, WELL_W, WELL_H);
    let well = hit(ui, px, "console-well", WELL_X, WELL_Y, WELL_W, WELL_H);
    // Clicking anywhere else gives the keyboard back to the player.
    if ui.input(|i| i.pointer.any_pressed()) && !well.contains_pointer() {
        app.term_focus = None;
    }
    let mut scroll = 0;
    if well.contains_pointer() {
        app.console_acc += ui.input(|i| i.raw_scroll_delta.y);
        let steps = (app.console_acc / 10.0) as i32;
        app.console_acc -= steps as f32 * 10.0;
        scroll = steps;
    }
    if scroll != 0 {
        app.scroll_console(scroll);
    }
    match app.tab {
        Tab::Logs => logs(app, px),
        Tab::Terminal => {
            if well.clicked() {
                app.focus_terminal();
            }
            terminal(app, px);
        }
    }
}

fn line_y(row: usize) -> i32 {
    WELL_Y + 2 + row as i32 * mono::CELL_H
}

fn status(px: &Px, s: &str, col: Color32) {
    px.text(WELL_X + 2, super::H - 7, &fit(s, 78), col);
}

fn logs(app: &App, px: &Px) {
    let buf = lock(&app.log);
    let top = app.log_top.map_or(buf.bottom(), |t| t.clamp(buf.first(), buf.bottom()));
    if buf.is_empty() {
        mono::draw(px, WELL_X + 2, line_y(0), "No log output.", pal::LCD_DIM);
    }
    for (i, row) in buf.view(top).enumerate() {
        let col = if row.stderr { pal::YELLOW } else { pal::LCD };
        mono::draw(px, WELL_X + 2, line_y(i), &row.text, col);
    }
    if app.log_top.is_some() {
        status(px, &format!("{} MORE BELOW - END TO FOLLOW", buf.bottom() - top), pal::YELLOW);
    } else {
        status(px, "FOLLOWING", pal::LCD_DIM);
    }
}

fn terminal(app: &App, px: &Px) {
    let Some(id) = app.selected.as_deref() else { return };
    let focused = app.term_focus.as_deref() == Some(id);
    let Some(session) = app.terms.get(id) else {
        let (msg, col) = if app.term_focus.is_some() {
            ("The container is gone. Click here to continue with this one.", pal::YELLOW)
        } else if app.selected_info().is_some_and(|c| c.state.is_up()) {
            ("Click here or press T to open a shell in this container.", pal::LCD)
        } else {
            ("This container is not running.", pal::LCD_DIM)
        };
        mono::draw(px, WELL_X + 2, line_y(0), msg, col);
        return;
    };
    let t = lock(&session.term);
    let screen = t.screen();
    let offset = screen.scrollback();
    let cursor = screen.cursor_position();
    let cursor_shown = !screen.hide_cursor() && t.ended.is_none();
    for r in 0..ROWS as u16 {
        for c in 0..COLS as u16 {
            let Some(cell) = screen.cell(r, c) else { continue };
            let (x, y) = (WELL_X + 2 + c as i32 * mono::CELL_W, line_y(r as usize));
            let mut fg = color(cell.fgcolor(), cell.bold(), pal::LCD);
            let mut bg = color(cell.bgcolor(), false, pal::LCD_BG);
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
            }
            let at_cursor = cursor_shown && (r as usize).checked_sub(offset) == Some(cursor.0 as usize) && c == cursor.1;
            if at_cursor && focused {
                (fg, bg) = (pal::LCD_BG, pal::LCD);
            }
            if bg != pal::LCD_BG {
                px.fill(x, y, mono::CELL_W, mono::CELL_H, bg);
            }
            if at_cursor && !focused {
                px.fill(x, y + 8, 5, 1, pal::LCD);
            }
            if let Some(ch) = cell.contents().chars().next() {
                mono::draw_char(px, x, y, ch, fg);
            }
            if cell.underline() {
                px.fill(x, y + 8, mono::CELL_W, 1, fg);
            }
        }
    }
    if let Some(end) = &t.ended {
        status(px, &format!("{} - CLICK OR PRESS ENTER TO REOPEN", end.to_uppercase()), pal::YELLOW);
    } else if offset > 0 {
        status(px, &format!("SCROLLED BACK {offset}"), pal::YELLOW);
    } else if focused {
        status(px, "KEYS GO TO THE SHELL - SHIFT+ESC TO RELEASE", pal::LCD);
    } else {
        status(px, "CLICK OR PRESS T TO TYPE", pal::LCD_DIM);
    }
}

fn color(c: vt100::Color, bold: bool, default: Color32) -> Color32 {
    const BASE: [(u8, u8, u8); 16] = [
        (0x30, 0x30, 0x30), (0xff, 0x40, 0x30), (0x00, 0xd0, 0x00), (0xe0, 0xc0, 0x20),
        (0x60, 0x80, 0xff), (0xe0, 0x60, 0xe0), (0x20, 0xd0, 0xd0), (0xc8, 0xc8, 0xc8),
        (0x78, 0x78, 0x78), (0xff, 0x78, 0x68), (0x60, 0xff, 0x60), (0xff, 0xf0, 0x60),
        (0x90, 0xa8, 0xff), (0xff, 0x90, 0xff), (0x70, 0xff, 0xff), (0xff, 0xff, 0xff),
    ];
    let (r, g, b) = match c {
        vt100::Color::Default => return if bold { pal::WHITE } else { default },
        vt100::Color::Rgb(r, g, b) => (r, g, b),
        vt100::Color::Idx(i @ 0..=7) if bold => BASE[i as usize + 8],
        vt100::Color::Idx(i @ 0..=15) => BASE[i as usize],
        vt100::Color::Idx(i @ 16..=231) => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            (level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        vt100::Color::Idx(i) => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
    };
    Color32::from_rgb(r, g, b)
}

/// Bytes a terminal sends for an input event, if any.
pub fn encode(e: &Event, mods: Modifiers, app_cursor: bool, bracketed: bool) -> Option<Vec<u8>> {
    match e {
        Event::Text(s) | Event::Ime(egui::ImeEvent::Commit(s)) => {
            let mut out = if mods.alt { vec![0x1b] } else { Vec::new() };
            out.extend(s.bytes());
            Some(out)
        }
        // The window layer turns Ctrl+C / X / V into these. Ctrl+Shift+C must not interrupt.
        Event::Copy => (!mods.shift).then(|| vec![0x03]),
        Event::Cut => Some(vec![0x18]),
        Event::Paste(s) => {
            let s: String = s.replace("\r\n", "\r").replace('\n', "\r").replace('\x1b', "");
            Some(if bracketed { format!("\x1b[200~{s}\x1b[201~") } else { s }.into_bytes())
        }
        Event::Key { key, pressed: true, modifiers: m, .. } => encode_key(*key, *m, app_cursor),
        _ => None,
    }
}

fn encode_key(key: Key, m: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    if m.ctrl {
        let name = key.name();
        if let ([c @ b'A'..=b'Z'], false) = (name.as_bytes(), m.alt) {
            return Some(vec![c & 0x1f]);
        }
        match key {
            Key::Space => return Some(vec![0]),
            Key::OpenBracket => return Some(vec![0x1b]),
            Key::Backslash => return Some(vec![0x1c]),
            Key::CloseBracket => return Some(vec![0x1d]),
            Key::Slash => return Some(vec![0x1f]),
            _ => {}
        }
    }
    // Arrows, Home and End: plain, application-cursor or with a modifier code.
    let code = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.ctrl as u8;
    let cursor = |c: char| match (code, app_cursor) {
        (1, true) => format!("\x1bO{c}"),
        (1, false) => format!("\x1b[{c}"),
        _ => format!("\x1b[1;{code}{c}"),
    };
    let s = match key {
        Key::Enter => "\r".into(),
        Key::Tab if m.shift => "\x1b[Z".into(),
        Key::Tab => "\t".into(),
        Key::Backspace if m.alt => "\x1b\x7f".into(),
        Key::Backspace => "\x7f".into(),
        Key::Escape => "\x1b".into(),
        Key::ArrowUp => cursor('A'),
        Key::ArrowDown => cursor('B'),
        Key::ArrowRight => cursor('C'),
        Key::ArrowLeft => cursor('D'),
        Key::Home => cursor('H'),
        Key::End => cursor('F'),
        Key::Insert => "\x1b[2~".into(),
        Key::Delete => "\x1b[3~".into(),
        Key::PageUp => "\x1b[5~".into(),
        Key::PageDown => "\x1b[6~".into(),
        Key::F1 => "\x1bOP".into(),
        Key::F2 => "\x1bOQ".into(),
        Key::F3 => "\x1bOR".into(),
        Key::F4 => "\x1bOS".into(),
        Key::F5 => "\x1b[15~".into(),
        Key::F6 => "\x1b[17~".into(),
        Key::F7 => "\x1b[18~".into(),
        Key::F8 => "\x1b[19~".into(),
        Key::F9 => "\x1b[20~".into(),
        Key::F10 => "\x1b[21~".into(),
        Key::F11 => "\x1b[23~".into(),
        Key::F12 => "\x1b[24~".into(),
        _ => return None,
    };
    Some(s.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_encoding() {
        let none = Modifiers::NONE;
        let enc = |k, m| encode_key(k, m, false).map(|b| String::from_utf8(b).unwrap());
        assert_eq!(enc(Key::D, Modifiers::CTRL).as_deref(), Some("\x04"));
        assert_eq!(enc(Key::D, none), None, "letters arrive as text");
        assert_eq!(enc(Key::Delete, none).as_deref(), Some("\x1b[3~"));
        assert_eq!(enc(Key::ArrowLeft, Modifiers::CTRL).as_deref(), Some("\x1b[1;5D"));
        assert_eq!(encode_key(Key::ArrowUp, none, true).unwrap(), b"\x1bOA");
        assert_eq!(encode(&Event::Copy, none, false, false).unwrap(), b"\x03");
        let paste = Event::Paste("a\nb\x1b[201~".into());
        assert_eq!(encode(&paste, none, false, true).unwrap(), b"\x1b[200~a\rb[201~\x1b[201~");
    }
}
