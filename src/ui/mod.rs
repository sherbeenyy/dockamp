pub mod details;
pub mod player;
pub mod playlist;

use eframe::egui::{Response, Sense, Ui, ViewportCommand};

use crate::skin::{pal, Px};

pub const W: i32 = 275;
pub const PLAYER_H: i32 = 116;
pub const DETAILS_H: i32 = 100;
pub const PLAYLIST_H: i32 = 140;
pub const H: i32 = PLAYER_H + DETAILS_H + PLAYLIST_H;

pub fn hit(ui: &Ui, px: &Px, id: &str, x: i32, y: i32, w: i32, h: i32) -> Response {
    ui.interact(px.rect_of(x, y, w, h), ui.id().with(id).with(px.origin.y as i32), Sense::click())
}

/// Title bar drag-to-move (everything but the right-side buttons area).
pub fn title_drag(ui: &Ui, px: &Px, id: &str) {
    let r = ui.interact(
        px.rect_of(0, 0, W - 40, 14),
        ui.id().with(id),
        Sense::click_and_drag(),
    );
    if r.drag_started() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// A raised chrome button with a bitmap icon; sunken while held. Returns clicked.
#[allow(clippy::too_many_arguments)]
pub fn button(ui: &Ui, px: &Px, id: &str, x: i32, y: i32, w: i32, h: i32, icon: &[&str]) -> bool {
    let r = hit(ui, px, id, x, y, w, h);
    let down = r.is_pointer_button_down_on();
    px.bevel(x, y, w, h, pal::BUTTON, !down);
    let iw = icon.first().map_or(0, |r| r.len() as i32);
    let ih = icon.len() as i32;
    let off = down as i32;
    px.bitmap(x + (w - iw) / 2 + off, y + (h - ih) / 2 + off, icon, pal::ICON);
    r.clicked()
}
