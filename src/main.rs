mod docker;
mod model;
mod skin;
mod ui;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Vec2, ViewportCommand};
use tokio::sync::mpsc::UnboundedSender;

use docker::{Command, StatsSample, Update};
use model::{ContainerInfo, Row, State};
use skin::{Canvas, Px};
use ui::player::{Transport, SPECTRUM_BARS};

const DEFAULT_SCALE: u8 = 2;

fn window_size(scale: u8) -> Vec2 {
    Vec2::new(ui::W as f32, ui::H as f32) * scale as f32
}

pub struct App {
    cmd: UnboundedSender<Command>,
    rx: Receiver<Update>,
    pub containers: Vec<ContainerInfo>,
    pub rows: Vec<Row>,
    pub selected: Option<String>,
    /// Name of the selection, so it survives the container being recreated with a new ID.
    selected_name: Option<String>,
    /// Container name currently being rebuilt (one at a time).
    building: Option<String>,
    pub stats: Option<StatsSample>,
    pub cpu_hist: VecDeque<f32>,
    /// Marquee override: (message, since, still in progress)
    flash: Option<(String, Instant, bool)>,
    pub error: Option<String>,
    pub scale: u8,
    pub scroll: usize,
    pub scroll_acc: f32,
    pub follow_selection: bool,
    pub marquee_t0: Instant,
    canvas: RefCell<Canvas>,
    tex: Option<egui::TextureHandle>,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        let (tx, rx) = channel();
        let ctx = cc.egui_ctx.clone();
        let cmd = docker::spawn(tx, move || ctx.request_repaint());
        Self {
            cmd,
            rx,
            containers: Vec::new(),
            rows: Vec::new(),
            selected: None,
            selected_name: None,
            building: None,
            stats: None,
            cpu_hist: VecDeque::with_capacity(SPECTRUM_BARS),
            flash: None,
            error: None,
            scale: DEFAULT_SCALE,
            scroll: 0,
            scroll_acc: 0.0,
            follow_selection: false,
            marquee_t0: Instant::now(),
            canvas: RefCell::new(Canvas::new(ui::W, ui::H)),
            tex: None,
        }
    }

    pub fn selected_info(&self) -> Option<&ContainerInfo> {
        let id = self.selected.as_deref()?;
        self.containers.iter().find(|c| c.id == id)
    }

    fn selected_index(&self) -> Option<usize> {
        let id = self.selected.as_deref()?;
        self.containers.iter().position(|c| c.id == id)
    }

    pub fn selected_row(&self) -> Option<usize> {
        let i = self.selected_index()?;
        self.rows.iter().position(|r| *r == Row::Item(i))
    }

    fn set_selected(&mut self, id: Option<String>) {
        if id != self.selected {
            self.selected_name = id
                .as_deref()
                .and_then(|id| self.containers.iter().find(|c| c.id == id))
                .map(|c| c.name.clone());
            self.selected = id.clone();
            self.stats = None;
            self.cpu_hist.clear();
            self.marquee_t0 = Instant::now();
            let _ = self.cmd.send(Command::Select(id));
        }
    }

    pub fn select_index(&mut self, i: usize) {
        let id = self.containers.get(i).map(|c| c.id.clone());
        self.set_selected(id);
        self.follow_selection = true;
    }

    pub fn step(&mut self, _ctx: &egui::Context, delta: i32) {
        if self.containers.is_empty() {
            return;
        }
        let n = self.containers.len() as i32;
        let cur = self.selected_index().map_or(0, |i| i as i32);
        self.select_index((cur + delta).rem_euclid(n) as usize);
    }

    pub fn transport(&mut self, t: Transport) {
        let Some(c) = self.selected_info() else { return };
        let (id, state) = (c.id.clone(), c.state);
        match t.action(state) {
            Some(a) => {
                self.flash = Some((a.progress().into(), Instant::now(), true));
                let _ = self.cmd.send(Command::Act(id, a));
            }
            None => {
                let msg = match state {
                    s if s.is_up() => "ALREADY RUNNING",
                    _ => "NOT RUNNING",
                };
                self.flash = Some((msg.into(), Instant::now(), false));
            }
        }
    }

    /// Shift+R: recreate (and build, if it has a Dockerfile) the selected compose service.
    pub fn rebuild(&mut self) {
        let Some(c) = self.selected_info() else { return };
        let msg = if let Some(b) = &self.building {
            format!("ALREADY REBUILDING {}", b.to_uppercase())
        } else if let Some(compose) = c.compose.clone() {
            let name = c.name.clone();
            self.flash = Some((format!("BUILDING {}...", compose.service), Instant::now(), true));
            self.building = Some(name.clone());
            let _ = self.cmd.send(Command::Rebuild(name, compose));
            return;
        } else {
            "NOT A COMPOSE CONTAINER - CANNOT REBUILD".into()
        };
        self.flash = Some((msg, Instant::now(), false));
    }

    pub fn cycle_scale(&mut self, ctx: &egui::Context) {
        self.scale = self.scale % 3 + 1;
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(window_size(self.scale)));
    }

    pub fn marquee_text(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        if let Some((m, _, _)) = &self.flash {
            return m.clone();
        }
        match (self.selected_index(), self.selected_info()) {
            (Some(i), Some(c)) => format!("{}. {} - {}", i + 1, c.name, c.image).to_uppercase(),
            _ if self.containers.is_empty() => "NO CONTAINERS - DOCKAMP".into(),
            _ => "DOCKAMP".into(),
        }
    }

    fn drain(&mut self) {
        while let Ok(u) = self.rx.try_recv() {
            match u {
                Update::Snapshot(mut v) => {
                    self.error = None;
                    self.rows = model::arrange(&mut v);
                    self.containers = v;
                    if self.selected_index().is_none() {
                        // Recreated (new ID, same name)? Keep it selected; else fall back to the first.
                        let same_name = self.selected_name.as_deref().and_then(|n| {
                            self.containers.iter().find(|c| c.name == n).map(|c| c.id.clone())
                        });
                        let next = same_name.or_else(|| self.containers.first().map(|c| c.id.clone()));
                        self.set_selected(next);
                    }
                }
                Update::Stats(s) => {
                    if self.selected.as_deref() == Some(s.id.as_str()) {
                        if self.cpu_hist.len() == SPECTRUM_BARS {
                            self.cpu_hist.pop_front();
                        }
                        self.cpu_hist.push_back(s.cpu);
                        self.stats = Some(s);
                    }
                }
                Update::ActionDone(msg) => {
                    if msg.starts_with("REBUIL") {
                        self.building = None;
                    }
                    self.flash = Some((msg, Instant::now(), false));
                }
                Update::Progress(line) => self.flash = Some((line, Instant::now(), true)),
                Update::Error(e) => self.error = Some(e),
            }
        }
        if let Some((_, t, false)) = &self.flash {
            if t.elapsed() > Duration::from_millis(2500) {
                self.flash = None;
                self.marquee_t0 = Instant::now();
            }
        }
    }

    fn keys(&mut self, ctx: &egui::Context) {
        let pressed = |k| ctx.input(|i| i.key_pressed(k));
        if pressed(Key::Z) || pressed(Key::ArrowUp) {
            self.step(ctx, -1);
        }
        if pressed(Key::B) || pressed(Key::ArrowDown) {
            self.step(ctx, 1);
        }
        if pressed(Key::X) {
            self.transport(Transport::Play);
        }
        if pressed(Key::C) {
            self.transport(Transport::Pause);
        }
        if pressed(Key::V) {
            self.transport(Transport::Stop);
        }
        if pressed(Key::R) {
            if ctx.input(|i| i.modifiers.shift) {
                self.rebuild();
            } else {
                self.transport(Transport::Restart);
            }
        }
        if pressed(Key::D) {
            self.cycle_scale(ctx);
        }
    }

    /// Repaint cadence: fast only while something animates, else once a second.
    fn next_tick(&self) -> Duration {
        let animating = self.flash.is_some()
            || self.marquee_text().chars().count() > 25
            || self.selected_info().is_some_and(|c| c.state == State::Paused);
        if animating {
            Duration::from_millis(200)
        } else {
            Duration::from_millis(1000)
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain();
        self.keys(ctx);
        let canvas = std::mem::replace(&mut self.canvas, RefCell::new(Canvas::new(0, 0)));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let origin = ui.max_rect().min;
            let px = Px { canvas: &canvas, oy: 0, origin, s: self.scale as f32 };
            ui::player::draw(self, ui, &px);
            ui::details::draw(self, ui, &px.at(ui::PLAYER_H));
            ui::playlist::draw(self, ui, &px.at(ui::PLAYER_H + ui::DETAILS_H));

            // One texture upload per frame, scaled with nearest-neighbour for crisp pixels.
            let image = egui::ColorImage {
                size: [ui::W as usize, ui::H as usize],
                pixels: canvas.borrow().pixels.clone(),
            };
            let tex = match &mut self.tex {
                Some(t) => {
                    t.set(image, egui::TextureOptions::NEAREST);
                    t
                }
                None => self.tex.insert(ctx.load_texture("skin", image, egui::TextureOptions::NEAREST)),
            };
            let rect = egui::Rect::from_min_size(origin, window_size(self.scale));
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(tex.id(), rect, uv, egui::Color32::WHITE);
        });
        self.canvas = canvas;
        ctx.request_repaint_after(self.next_tick());
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DockAmp")
            .with_app_id("dockamp")
            .with_inner_size(window_size(DEFAULT_SCALE))
            .with_decorations(false)
            .with_resizable(false),
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };
    eframe::run_native("DockAmp", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
