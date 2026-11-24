//! Main "now playing" panel: uptime LCD, marquee, CPU spectrum, meters, transport.

use chrono::Utc;
use eframe::egui::{Color32, Ui, ViewportCommand};

use super::{button, title_drag, W};
use crate::docker::Action;
use crate::model::{self, State};
use crate::skin::{font, icon, lcd, pal, Px};
use crate::App;

const MARQUEE_CHARS: usize = 25;
pub const SPECTRUM_BARS: usize = 19;

pub fn draw(app: &mut App, ui: &Ui, px: &Px) {
    px.panel(W, super::PLAYER_H, "DOCKAMP");
    title_drag(ui, px, "title-player");

    // Title buttons: scale, minimise, close.
    if button(ui, px, "scale", W - 38, 3, 11, 9, &[]) {
        app.cycle_scale(ui.ctx());
    }
    px.text(W - 35, 5, &app.scale.to_string(), pal::ICON);
    if button(ui, px, "min", W - 25, 3, 9, 9, icon::MIN) {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }
    if button(ui, px, "close", W - 14, 3, 9, 9, icon::CLOSE) {
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
    }

    let now = Utc::now();
    let sel = app.selected_info().cloned();

    // --- LCD well: status glyph, uptime, spectrum -------------------------
    px.well(9, 22, 95, 42);
    let (glyph, gc): (&[&str], Color32) = match sel.as_ref().map(|c| c.state) {
        Some(State::Running) | Some(State::Restarting) => (icon::STATE_PLAY, pal::LCD),
        Some(State::Paused) => (icon::STATE_PAUSE, pal::YELLOW),
        Some(_) => (icon::STATE_STOP, pal::LCD_DIM),
        None => (icon::STATE_STOP, pal::LCD_GHOST),
    };
    px.bitmap(13, 27, glyph, gc);

    let (time, lit) = match &sel {
        Some(c) if c.state.is_up() => {
            let secs = c.started_at.map_or(0, |t| (now - t).num_seconds());
            let blink = c.state == State::Paused && (now.timestamp_subsec_millis() / 500) % 2 == 1;
            (model::lcd_time(secs), !blink)
        }
        Some(c) => match c.finished_at {
            // Stopped: show time since exit, with a minus like "remaining time".
            Some(t) => (format!("-{}", model::lcd_time((now - t).num_seconds())), true),
            None => ("00:00:00".into(), false),
        },
        None => ("00:00:00".into(), false),
    };
    lcd::draw(px, 100 - lcd::width(&time), 26, &time, lit);

    // CPU history as spectrum bars, newest on the right.
    let n = app.cpu_hist.len();
    for i in 0..SPECTRUM_BARS {
        let x = 22 + i as i32 * 4;
        let v = if i + n >= SPECTRUM_BARS { app.cpu_hist[i + n - SPECTRUM_BARS] } else { 0.0 };
        let h = if v > 0.0 { ((v.min(100.0) / 100.0) * 16.0).ceil().max(1.0) as i32 } else { 0 };
        for row in 0..h {
            let c = match row {
                0..=8 => pal::LCD,
                9..=12 => pal::YELLOW,
                _ => pal::RED,
            };
            px.fill(x, 61 - row, 3, 1, c);
        }
    }

    // --- Marquee ------------------------------------------------------------
    px.well(110, 23, 155, 10);
    let text = app.marquee_text();
    let shown = marquee(&text, app.marquee_t0.elapsed().as_millis());
    let mc = if app.error.is_some() { pal::RED } else { pal::LCD };
    px.text(112, 25, &shown, mc);

    // --- Small readouts -----------------------------------------------------
    let st = app.stats.as_ref();
    let up = sel.as_ref().is_some_and(|c| c.state.is_up());
    let cpu = st.filter(|_| up).map(|s| s.cpu);
    let mem = st.filter(|_| up).map(|s| (s.mem, s.mem_limit));

    px.well(110, 38, 30, 9);
    px.text(112, 40, &cpu.map_or("--".into(), |c| format!("{:.0}%", c)), pal::LCD);
    px.text(143, 40, "CPU", pal::CHROME_TEXT);
    px.well(165, 38, 30, 9);
    px.text(167, 40, &mem.map_or("--".into(), |(m, _)| model::bytes(m)), pal::LCD);
    px.text(198, 40, "MEM", pal::CHROME_TEXT);

    px.well(220, 38, 45, 9);
    if let Some(c) = &sel {
        let (label, col) = match c.state {
            State::Running => match c.health.as_deref() {
                Some("unhealthy") => ("SICK", pal::RED),
                _ => ("RUNNING", pal::LCD),
            },
            State::Paused => ("PAUSED", pal::YELLOW),
            State::Restarting => ("RESTART", pal::YELLOW),
            State::Created => ("CREATED", pal::LCD_DIM),
            State::Exited => ("EXITED", pal::LCD_DIM),
            State::Dead => ("DEAD", pal::RED),
        };
        px.text(222, 40, label, col);
    }

    // --- Meters: "volume" = CPU, "balance" = MEM% ---------------------------
    let mem_pct = mem.map_or(0.0, |(m, l)| if l > 0 { m as f32 / l as f32 * 100.0 } else { 0.0 });
    slider(px, 110, 53, 68, cpu.unwrap_or(0.0) / 100.0, "CPU");
    slider(px, 182, 53, 38, mem_pct / 100.0, "MEM");

    // --- Seek bar = memory used vs limit -------------------------------------
    let frac = (mem_pct / 100.0).clamp(0.0, 1.0);
    px.bevel(16, 68, 248, 10, pal::CHROME_LO, false);
    px.fill(17, 69, ((246.0 * frac) as i32).max(0), 8, Color32::from_rgb(0x1c, 0x50, 0x1c));
    let label = mem.map_or("MEMORY  --".into(), |(m, l)| {
        format!("MEMORY  {} / {}  ({:.1}%)", model::bytes(m), model::bytes(l), mem_pct)
    });
    px.text(16 + 248 - 4 - font::width(&label), 71, &label, pal::CHROME_TEXT);
    let tx = 16 + ((248 - 29) as f32 * frac) as i32;
    px.bevel(tx, 68, 29, 10, pal::BUTTON, true);

    // --- Transport -------------------------------------------------------------
    let y = 84;
    if button(ui, px, "prev", 16, y, 23, 18, icon::PREV) {
        app.step(ui.ctx(), -1);
    }
    if button(ui, px, "play", 39, y, 23, 18, icon::PLAY) {
        app.transport(Transport::Play);
    }
    if button(ui, px, "pause", 62, y, 23, 18, icon::PAUSE) {
        app.transport(Transport::Pause);
    }
    if button(ui, px, "stop", 85, y, 23, 18, icon::STOP) {
        app.transport(Transport::Stop);
    }
    if button(ui, px, "next", 108, y, 23, 18, icon::NEXT) {
        app.step(ui.ctx(), 1);
    }
    if button(ui, px, "restart", 135, y, 23, 18, icon::RESTART) {
        app.transport(Transport::Restart);
    }

    let up_n = app.containers.iter().filter(|c| c.state.is_up()).count();
    let summary = format!("{}/{} UP", up_n, app.containers.len());
    px.text(W - 12 - font::width(&summary), y + 2, &summary, pal::CHROME_TEXT);
    px.text(W - 12 - font::width("DOCKAMP"), y + 11, "DOCKAMP", pal::TITLE_A);
}

#[derive(Clone, Copy)]
pub enum Transport {
    Play,
    Pause,
    Stop,
    Restart,
}

impl Transport {
    pub fn action(self, state: State) -> Option<Action> {
        match (self, state) {
            (Transport::Play, State::Paused) => Some(Action::Unpause),
            (Transport::Play, State::Running | State::Restarting) => None,
            (Transport::Play, _) => Some(Action::Start),
            (Transport::Pause, State::Running) => Some(Action::Pause),
            (Transport::Pause, State::Paused) => Some(Action::Unpause),
            (Transport::Pause, _) => None,
            (Transport::Stop, s) if s.is_up() => Some(Action::Stop),
            (Transport::Stop, _) => None,
            (Transport::Restart, _) => Some(Action::Restart),
        }
    }
}

/// Read-only slider: sunken track, level-coloured fill, raised thumb.
fn slider(px: &Px, x: i32, y: i32, w: i32, frac: f32, label: &str) {
    let frac = frac.clamp(0.0, 1.0);
    px.bevel(x, y, w, 8, pal::CHROME_LO, false);
    let col = if frac < 0.5 {
        pal::LCD_DIM
    } else if frac < 0.8 {
        Color32::from_rgb(0x90, 0x88, 0x10)
    } else {
        Color32::from_rgb(0xa0, 0x20, 0x10)
    };
    px.fill(x + 1, y + 1, ((w - 2) as f32 * frac) as i32, 6, col);
    px.text(x + w - 3 - font::width(label), y + 2, label, pal::CHROME_TEXT);
    let tx = x + ((w - 10) as f32 * frac) as i32;
    px.bevel(tx, y - 1, 10, 10, pal::BUTTON, true);
}

fn marquee(text: &str, ms: u128) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= MARQUEE_CHARS {
        return text.to_string();
    }
    let loop_text: Vec<char> = chars.iter().copied().chain("  ***  ".chars()).collect();
    let off = (ms / 200) as usize % loop_text.len();
    (0..MARQUEE_CHARS).map(|i| loop_text[(off + i) % loop_text.len()]).collect()
}
