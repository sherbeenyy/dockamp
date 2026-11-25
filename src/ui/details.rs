//! Inspector panel (sits where an equalizer would): static facts about the selection.

use chrono::Local;
use eframe::egui::{CursorIcon, Ui};

use super::{hit, title_drag, W};
use crate::model::fit;
use crate::skin::{pal, Px};
use crate::App;

pub fn draw(app: &mut App, ui: &Ui, px: &Px) {
    px.panel(W, super::DETAILS_H, "DOCKAMP INSPECTOR");
    title_drag(ui, px, "title-details");

    let c = app.selected_info();
    let dash = || "--".to_string();
    let fields: [(&str, String); 7] = [
        ("ID", c.map_or_else(dash, |c| c.short_id().to_string())),
        ("IMAGE", c.map_or_else(dash, |c| c.image.clone())),
        ("COMMAND", c.map_or_else(dash, |c| c.command.clone())),
        (
            "PORTS",
            c.map_or_else(dash, |c| if c.ports.is_empty() { "none".into() } else { c.ports.join(", ") }),
        ),
        (
            "HEALTH",
            c.map_or_else(dash, |c| c.health.clone().unwrap_or_else(|| "no healthcheck".into())),
        ),
        (
            "RESTARTS",
            c.map_or_else(dash, |c| format!("{}   EXIT CODE {}", c.restart_count, c.exit_code)),
        ),
        (
            "CREATED",
            c.and_then(|c| c.created)
                .map_or_else(dash, |t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string()),
        ),
    ];

    // The ID is clickable: copies it to the clipboard.
    let id_hit = c.is_some().then(|| {
        hit(ui, px, "copy-id", 62, 17, W - 8 - 62, 10).on_hover_cursor(CursorIcon::PointingHand)
    });
    let id_hovered = id_hit.as_ref().is_some_and(|r| r.hovered());

    let max = ((W - 8 - 62 - 4) / 6) as usize;
    for (i, (label, value)) in fields.iter().enumerate() {
        let y = 18 + i as i32 * 11;
        // Label as a small raised "EQ band" plate.
        px.bevel(8, y - 1, 50, 10, pal::BUTTON, true);
        px.text(11, y + 1, label, pal::CHROME_TEXT);
        px.well(62, y, W - 8 - 62, 8);
        let col = match (*label, value.as_str()) {
            ("ID", _) if id_hovered => pal::WHITE,
            ("HEALTH", "unhealthy") => pal::RED,
            ("HEALTH", "starting") => pal::YELLOW,
            _ => pal::LCD,
        };
        px.text(64, y + 1, &fit(value, max), col);
    }

    if id_hit.is_some_and(|r| r.clicked()) {
        app.copy_id(ui.ctx());
    }
}
