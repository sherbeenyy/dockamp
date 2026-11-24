//! Playlist panel: every container as a "track", uptime as the duration.

use chrono::Utc;
use eframe::egui::Ui;

use super::{hit, title_drag, W};
use crate::model::{self, fit, Row, State};
use crate::skin::{font, pal, Px};
use crate::ui::player::Transport;
use crate::App;

pub const VISIBLE: usize = 14;
const ROW_H: i32 = 8;
const WELL_X: i32 = 8;
const WELL_Y: i32 = 16;
const WELL_W: i32 = W - 16;

pub fn draw(app: &mut App, ui: &Ui, px: &Px) {
    px.panel(W, super::PLAYLIST_H, "DOCKAMP PLAYLIST");
    title_drag(ui, px, "title-playlist");
    px.well(WELL_X, WELL_Y, WELL_W, VISIBLE as i32 * ROW_H);

    // Mouse wheel scrolling over the list.
    let well = hit(ui, px, "pl-well", WELL_X, WELL_Y, WELL_W, VISIBLE as i32 * ROW_H);
    if well.hovered() {
        app.scroll_acc += ui.input(|i| i.raw_scroll_delta.y);
        while app.scroll_acc >= 20.0 {
            app.scroll = app.scroll.saturating_sub(1);
            app.scroll_acc -= 20.0;
        }
        while app.scroll_acc <= -20.0 {
            app.scroll += 1;
            app.scroll_acc += 20.0;
        }
    }
    let max_scroll = app.rows.len().saturating_sub(VISIBLE);
    if app.follow_selection {
        if let Some(r) = app.selected_row() {
            if r < app.scroll {
                app.scroll = r;
            } else if r >= app.scroll + VISIBLE {
                app.scroll = r + 1 - VISIBLE;
            }
        }
        app.follow_selection = false;
    }
    app.scroll = app.scroll.min(max_scroll);

    let now = Utc::now();
    let mut clicked: Option<(usize, bool)> = None;
    for vis in 0..VISIBLE {
        let Some(row) = app.rows.get(app.scroll + vis) else { break };
        let y = WELL_Y + vis as i32 * ROW_H;
        match row {
            Row::Header(p) => {
                let t = format!("== {} ==", p.to_uppercase());
                px.text(WELL_X + 3, y + 2, &fit(&t, 42), pal::HEADER);
            }
            Row::Item(i) => {
                let c = &app.containers[*i];
                let selected = app.selected.as_deref() == Some(c.id.as_str());
                if selected {
                    px.fill(WELL_X, y, WELL_W, ROW_H, pal::SELECT_BG);
                }
                let dur = match c.state {
                    s if s.is_up() => model::track_time(c.started_at.map_or(0, |t| (now - t).num_seconds())),
                    State::Created => "new".into(),
                    _ => match c.finished_at {
                        Some(t) => format!("-{}", model::ago((now - t).num_seconds())),
                        None => "--".into(),
                    },
                };
                let col = if selected {
                    pal::WHITE
                } else {
                    match c.state {
                        State::Running => pal::LCD,
                        State::Paused | State::Restarting => pal::YELLOW,
                        State::Dead => pal::RED,
                        _ => pal::LCD_DIM,
                    }
                };
                let max = ((WELL_W - 6 - font::width(&dur) - 8) / font::CELL) as usize;
                let name = fit(&format!("{}. {}", i + 1, c.name), max);
                px.text(WELL_X + 3, y + 2, &name, col);
                px.text(WELL_X + WELL_W - 3 - font::width(&dur), y + 2, &dur, col);

                let r = hit(ui, px, &format!("row-{}", c.id), WELL_X, y, WELL_W, ROW_H);
                if r.double_clicked() {
                    clicked = Some((*i, true));
                } else if r.clicked() {
                    clicked = Some((*i, false));
                }
            }
        }
    }
    if let Some((i, double)) = clicked {
        app.select_index(i);
        if double {
            app.transport(Transport::Play);
        }
    }

    // Bottom status bar.
    let by = super::PLAYLIST_H - 8;
    let up = app.containers.iter().filter(|c| c.state.is_up()).count();
    px.text(10, by, &format!("{} RUNNING  {} TOTAL", up, app.containers.len()), pal::CHROME_TEXT);
    if app.rows.len() > VISIBLE {
        let t = format!("{}-{}/{}", app.scroll + 1, (app.scroll + VISIBLE).min(app.rows.len()), app.rows.len());
        px.text(W - 10 - font::width(&t), by, &t, pal::CHROME_TEXT);
    } else {
        let t = "Z X C V B";
        px.text(W - 10 - font::width(t), by, t, pal::LCD_DIM);
    }
}
