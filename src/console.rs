//! Log buffer and terminal emulator state, shared between the Docker worker (which fills
//! them) and the UI (which draws them). Both are bounded, so a chatty container can't grow
//! memory even while the window isn't being drawn.

use std::collections::VecDeque;
use std::num::NonZeroU16;
use std::sync::{Arc, Mutex, MutexGuard};

pub const COLS: usize = 80;
pub const ROWS: usize = 35;
const MAX_LOG_ROWS: usize = 5000;
const MAX_PARTIAL: usize = 16 * 1024;
const SCROLLBACK: usize = 2000;

pub type Shared<T> = Arc<Mutex<T>>;

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRow {
    pub text: String,
    pub stderr: bool,
}

/// Log output, cleaned up and wrapped to the console width.
#[derive(Default)]
pub struct LogBuf {
    rows: VecDeque<LogRow>,
    /// Absolute number of `rows[0]`, so a scrolled-up view stays put as rows arrive.
    first: u64,
    /// Unterminated last line of stdout / stderr.
    partial: [Vec<u8>; 2],
}

impl LogBuf {
    pub fn clear(&mut self) {
        self.first += self.rows.len() as u64;
        self.rows.clear();
        self.partial = Default::default();
    }

    pub fn push(&mut self, bytes: &[u8], stderr: bool) {
        for piece in bytes.split_inclusive(|b| *b == b'\n') {
            let part = &mut self.partial[stderr as usize];
            part.extend_from_slice(piece);
            if piece.ends_with(b"\n") || part.len() > MAX_PARTIAL {
                self.flush(stderr);
            }
        }
    }

    /// Ends of stream: show unterminated lines too.
    pub fn finish(&mut self) {
        for stderr in [false, true] {
            if !self.partial[stderr as usize].is_empty() {
                self.flush(stderr);
            }
        }
    }

    pub fn note(&mut self, text: &str) {
        self.finish();
        self.add(format!("-- {text} --"), true);
    }

    fn flush(&mut self, stderr: bool) {
        let raw = std::mem::take(&mut self.partial[stderr as usize]);
        let line = clean(&String::from_utf8_lossy(&raw));
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            self.add(String::new(), stderr);
        }
        for row in chars.chunks(COLS) {
            self.add(row.iter().collect(), stderr);
        }
    }

    fn add(&mut self, text: String, stderr: bool) {
        self.rows.push_back(LogRow { text, stderr });
        if self.rows.len() > MAX_LOG_ROWS {
            self.rows.pop_front();
            self.first += 1;
        }
    }

    /// Absolute top row of the view when following the newest output.
    pub fn bottom(&self) -> u64 {
        self.first + self.rows.len().saturating_sub(ROWS) as u64
    }

    pub fn first(&self) -> u64 {
        self.first
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The rows visible from absolute row `top`.
    pub fn view(&self, top: u64) -> impl Iterator<Item = &LogRow> {
        self.rows.iter().skip(top.saturating_sub(self.first) as usize).take(ROWS)
    }
}

/// Strips escape sequences and control characters, expands tabs, and applies `\r` overwrites.
fn clean(line: &str) -> String {
    let line = line.trim_end_matches(['\r', '\n']);
    let line = line.rsplit('\r').next().unwrap_or("");
    let mut out = String::new();
    let mut width = 0;
    let mut it = line.chars().peekable();
    while let Some(ch) = it.next() {
        match ch {
            '\x1b' => match it.next() {
                Some('[') => {
                    while it.next().is_some_and(|c| !('@'..='~').contains(&c)) {}
                }
                Some(']') => {
                    while let Some(c) = it.next() {
                        if c == '\x07' || (c == '\x1b' && it.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => {
                let n = 8 - width % 8;
                out.extend(std::iter::repeat_n(' ', n));
                width += n;
            }
            c if c.is_control() => {}
            c => {
                out.push(c);
                width += 1;
            }
        }
    }
    out
}

#[derive(Default)]
pub struct Events {
    /// PID of the shell inside the container, announced by the launch script.
    pid: Option<u32>,
    reply: Vec<u8>,
}

impl vt100::Callbacks for Events {
    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        // Only the launch script's (first) announcement counts.
        if let ([b"777", p], None) = (params, self.pid) {
            self.pid = std::str::from_utf8(p)
                .ok()
                .and_then(|p| p.strip_prefix("dockamp="))
                .and_then(|p| p.parse().ok());
        }
    }

    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        _: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        match (i1, params, c) {
            (None, [[6]], 'n') => {
                let (r, col) = screen.cursor_position();
                self.reply.extend(format!("\x1b[{};{}R", r + 1, col + 1).bytes());
            }
            (None, [[5]], 'n') => self.reply.extend(b"\x1b[0n"),
            (None, _, 'c') => self.reply.extend(b"\x1b[?6c"),
            _ => {}
        }
    }
}

pub struct Term {
    parser: vt100::Parser<Events>,
    /// Why the session is over, once it is.
    pub ended: Option<String>,
}

impl Term {
    pub fn new() -> Self {
        let n = |v: usize| NonZeroU16::new(v as u16).expect("non-zero");
        Self {
            parser: vt100::Parser::new_with_callbacks(n(ROWS), n(COLS), SCROLLBACK, Events::default()),
            ended: None,
        }
    }

    /// Feeds output; returns bytes to send back (answers to terminal queries).
    pub fn process(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.parser.process(bytes);
        std::mem::take(&mut self.parser.callbacks_mut().reply)
    }

    pub fn pid(&self) -> Option<u32> {
        self.parser.callbacks().pid
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    /// Scrolls by `delta` rows; positive goes back in history.
    pub fn scroll(&mut self, delta: i32) {
        let s = self.parser.screen_mut();
        let n = (s.scrollback() as i64 + delta as i64).max(0) as usize;
        s.set_scrollback(n);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.parser.screen_mut().set_scrollback(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(b: &LogBuf) -> Vec<&str> {
        b.view(b.first()).map(|r| r.text.as_str()).collect()
    }

    #[test]
    fn log_lines_are_joined_cleaned_and_wrapped() {
        let mut b = LogBuf::default();
        b.push(b"hel", false);
        b.push(b"oops\n", true);
        b.push(b"lo\r\n\x1b[31mred\x1b[0m\ta\n10%\r20%\n", false);
        assert_eq!(texts(&b), ["oops", "hello", "red     a", "20%"]);
        b.clear();
        b.push("x".repeat(COLS + 5).as_bytes(), false);
        assert!(b.is_empty(), "unterminated");
        b.finish();
        assert_eq!(texts(&b), ["x".repeat(COLS), "x".repeat(5)]);
    }

    #[test]
    fn log_is_bounded_and_row_numbers_are_stable() {
        let mut b = LogBuf::default();
        for i in 0..MAX_LOG_ROWS + 10 {
            b.push(format!("{i}\n").as_bytes(), false);
        }
        assert_eq!(b.first(), 10);
        assert_eq!(b.view(100).next().unwrap().text, "100");
        assert_eq!(b.bottom(), (MAX_LOG_ROWS + 10 - ROWS) as u64);
    }

    #[test]
    fn terminal_reports_pid_once_and_answers_queries() {
        let mut t = Term::new();
        t.process(b"\x1b]777;dockamp=42\x07");
        t.process(b"\x1b]777;dockamp=1\x07");
        assert_eq!(t.pid(), Some(42));
        assert_eq!(t.process(b"ab\x1b[6n"), b"\x1b[1;3R");
    }

    #[test]
    fn terminal_scrollback_past_one_screen() {
        let mut t = Term::new();
        for i in 0..200 {
            t.process(format!("{i}\r\n").as_bytes());
        }
        t.scroll(100_000);
        assert_eq!(t.screen().cell(0, 0).unwrap().contents(), "0");
        t.scroll_to_bottom();
        assert_eq!(t.screen().scrollback(), 0);
    }
}
