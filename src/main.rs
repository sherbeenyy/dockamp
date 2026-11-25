mod console;
mod docker;
mod model;
mod session;
mod skin;
mod ui;

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Vec2, ViewportCommand};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use console::{lock, LogBuf, Shared, Term};
use docker::{Action, Command, StatsSample, Update};
use model::{Confirm, ContainerInfo, Row, State};
use skin::{Canvas, Px};
use ui::console::Tab;
use ui::player::{Transport, SPECTRUM_BARS};

const DEFAULT_SCALE: u8 = 2;

fn window_size(scale: u8, console: bool) -> Vec2 {
    Vec2::new(canvas_w(console) as f32, ui::H as f32) * scale as f32
}

fn canvas_w(console: bool) -> i32 {
    ui::W + if console { ui::console::CONSOLE_W } else { 0 }
}

/// An open shell. Dropping it hangs the shell up.
pub struct Session {
    pub term: Shared<Term>,
    input: UnboundedSender<Vec<u8>>,
}

pub struct App {
    /// None once the app is closing.
    cmd: Option<UnboundedSender<Command>>,
    worker_done: Receiver<()>,
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
    /// Container waiting for the second remove press.
    removing: Confirm,
    pub error: Option<String>,
    /// Scale in use: the chosen one, or less if the window wouldn't fit the monitor.
    pub scale: u8,
    chosen_scale: u8,
    console_open: bool,
    pub tab: Tab,
    pub log: Shared<LogBuf>,
    /// Absolute top log row while scrolled up; None follows new output.
    pub log_top: Option<u64>,
    logs_for: Option<String>,
    pub terms: HashMap<String, Session>,
    /// Container whose terminal has the keyboard. It is kept when the shell or container goes
    /// away, so what is still being typed never reaches the player's shortcuts.
    pub term_focus: Option<String>,
    pub console_acc: f32,
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
        // The skin has its own scaling, and Ctrl+- / Ctrl+0 belong to the shell.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let (cmd, worker_done) = docker::spawn(tx, move || ctx.request_repaint());
        Self {
            cmd: Some(cmd),
            worker_done,
            rx,
            containers: Vec::new(),
            rows: Vec::new(),
            selected: None,
            selected_name: None,
            building: None,
            stats: None,
            cpu_hist: VecDeque::with_capacity(SPECTRUM_BARS),
            flash: None,
            removing: Confirm::default(),
            error: None,
            scale: DEFAULT_SCALE,
            chosen_scale: DEFAULT_SCALE,
            console_open: false,
            tab: Tab::Logs,
            log: Arc::default(),
            log_top: None,
            logs_for: None,
            terms: HashMap::new(),
            term_focus: None,
            console_acc: 0.0,
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
            self.removing.cancel();
            self.stats = None;
            self.cpu_hist.clear();
            self.marquee_t0 = Instant::now();
            self.send(Command::Select(id));
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
                self.send(Command::Act(id, a));
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
            self.send(Command::Rebuild(name, compose));
            return;
        } else {
            "NOT A COMPOSE CONTAINER - CANNOT REBUILD".into()
        };
        self.flash = Some((msg, Instant::now(), false));
    }

    /// Del / eject: the first press asks, the second one force-removes the container.
    pub fn remove(&mut self) {
        let Some(c) = self.selected_info() else { return };
        let (id, name) = (c.id.clone(), c.name.clone());
        if self.building.as_deref() == Some(name.as_str()) {
            self.flash = Some(("REBUILDING - CANNOT REMOVE".into(), Instant::now(), false));
        } else if self.removing.press(&id, Instant::now()) {
            let a = Action::Remove;
            self.flash = Some((a.progress().into(), Instant::now(), true));
            self.send(Command::Act(id, a));
        }
        self.marquee_t0 = Instant::now();
    }

    /// Ctrl+C / click on the ID: puts the selected container's short ID on the clipboard.
    pub fn copy_id(&mut self, ctx: &egui::Context) {
        let Some(c) = self.selected_info() else { return };
        let id = c.short_id().to_string();
        self.removing.cancel();
        self.flash = Some((format!("COPIED {id}").to_uppercase(), Instant::now(), false));
        self.marquee_t0 = Instant::now();
        ctx.copy_text(id);
    }

    /// True while the selected container is waiting for the second remove press.
    pub fn remove_pending(&self) -> bool {
        self.selected.as_deref().is_some_and(|id| self.removing.pending(id, Instant::now()))
    }

    pub fn cycle_scale(&mut self, ctx: &egui::Context) {
        let next = self.scale % 3 + 1;
        self.chosen_scale = if next > self.max_scale(ctx) { 1 } else { next };
        self.resize(ctx);
    }

    /// Largest scale at which the window fits the monitor.
    fn max_scale(&self, ctx: &egui::Context) -> u8 {
        let Some(m) = ctx.input(|i| i.viewport().monitor_size).filter(|m| m.x > 0.0) else { return 3 };
        let fits = |s: &u8| window_size(*s, self.console_open).x <= m.x;
        (1..=3).rev().find(fits).unwrap_or(1)
    }

    fn resize(&mut self, ctx: &egui::Context) {
        self.scale = self.chosen_scale.min(self.max_scale(ctx));
        let size = window_size(self.scale, self.console_open);
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
    }

    fn send(&self, c: Command) {
        if let Some(cmd) = &self.cmd {
            let _ = cmd.send(c);
        }
    }

    pub fn open_console(&mut self, ctx: &egui::Context, tab: Tab) {
        if !self.console_open {
            self.console_open = true;
            self.resize(ctx);
        }
        self.tab = tab;
        self.removing.cancel();
        match tab {
            Tab::Logs => self.term_focus = None,
            Tab::Terminal => self.focus_terminal(),
        }
    }

    pub fn close_console(&mut self, ctx: &egui::Context) {
        self.console_open = false;
        self.term_focus = None;
        self.resize(ctx);
    }

    pub fn toggle_console(&mut self, ctx: &egui::Context) {
        if self.console_open {
            self.close_console(ctx);
        } else {
            self.open_console(ctx, Tab::Logs);
        }
    }

    /// `L` / `T`: show that tab; `L` on the open LOGS tab closes the console.
    fn console_key(&mut self, ctx: &egui::Context, tab: Tab) {
        if tab == Tab::Logs && self.console_open && self.tab == tab {
            self.close_console(ctx);
        } else {
            self.open_console(ctx, tab);
        }
    }

    /// Gives the keyboard to the selected container's shell, opening one if needed.
    pub fn focus_terminal(&mut self) {
        self.term_focus = None;
        let Some(id) = self.selected.clone() else { return };
        let live = self.terms.get(&id).is_some_and(|s| lock(&s.term).ended.is_none());
        if !live {
            if !self.selected_info().is_some_and(|c| c.state.is_up()) {
                return;
            }
            let (input, rx) = unbounded_channel();
            let term = Arc::new(Mutex::new(Term::new()));
            let reply = input.downgrade();
            self.send(Command::Shell { id: id.clone(), term: term.clone(), input: rx, reply });
            self.terms.insert(id.clone(), Session { term, input });
        }
        self.term_focus = Some(id);
    }

    /// Positive scrolls back.
    pub fn scroll_console(&mut self, delta: i32) {
        if self.tab == Tab::Logs {
            let buf = lock(&self.log);
            let top = self.log_top.unwrap_or(buf.bottom()) as i64 - delta as i64;
            let top = top.max(buf.first() as i64) as u64;
            self.log_top = (top < buf.bottom()).then_some(top);
        } else if let Some(s) = self.selected.as_ref().and_then(|id| self.terms.get(id)) {
            lock(&s.term).scroll(delta);
        }
    }

    /// Keeps the worker following the logs of whatever the LOGS tab shows.
    fn sync_logs(&mut self) {
        let want = self.selected.clone().filter(|_| self.console_open && self.tab == Tab::Logs);
        if want != self.logs_for {
            self.logs_for = want.clone();
            self.log_top = None;
            lock(&self.log).clear();
            // A fresh buffer, so a late write for the old container can't land in this one.
            self.log = Arc::default();
            self.send(Command::Logs(want, self.log.clone()));
        }
    }

    fn terminal_keys(&mut self, ctx: &egui::Context, focus: &str) {
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let pressed = |want: egui::Modifiers, k: Key| {
            events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. }
                if *key == k && modifiers.matches_exact(want)))
        };
        if pressed(egui::Modifiers::SHIFT, Key::Escape) {
            self.term_focus = None;
            return;
        }
        // Never type into a container the user didn't choose to type into.
        if self.selected.as_deref() != Some(focus) {
            return;
        }
        for (k, d) in [(Key::PageUp, console::ROWS as i32 / 2), (Key::PageDown, -(console::ROWS as i32 / 2))] {
            if pressed(egui::Modifiers::SHIFT, k) {
                self.scroll_console(d);
                return;
            }
        }
        let Some(s) = self.terms.get(focus) else { return };
        let mut t = lock(&s.term);
        if t.ended.is_some() {
            drop(t);
            if pressed(egui::Modifiers::NONE, Key::Enter) {
                self.focus_terminal();
                // Not running any more: still keep the keyboard.
                self.term_focus.get_or_insert_with(|| focus.to_string());
            }
            return;
        }
        let (app_cursor, bracketed) = (t.screen().application_cursor(), t.screen().bracketed_paste());
        let bytes: Vec<u8> =
            events.iter().filter_map(|e| ui::console::encode(e, mods, app_cursor, bracketed)).flatten().collect();
        if !bytes.is_empty() {
            t.scroll_to_bottom();
            let _ = s.input.send(bytes);
        }
    }

    pub fn marquee_text(&self) -> String {
        if let Some(e) = &self.error {
            return e.clone();
        }
        if self.remove_pending() {
            let name = self.selected_info().map_or("", |c| c.name.as_str());
            return format!("DEL AGAIN TO REMOVE {name}").to_uppercase();
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
                    let old_index = self.selected_index();
                    self.rows = model::arrange(&mut v);
                    self.containers = v;
                    let containers = &self.containers;
                    self.terms.retain(|id, _| containers.iter().any(|c| c.id == *id));
                    if self.selected_index().is_none() {
                        // Recreated (new ID, same name)? Keep it selected; else take its neighbour.
                        let same_name = self.selected_name.as_deref().and_then(|n| {
                            self.containers.iter().find(|c| c.name == n).map(|c| c.id.clone())
                        });
                        let next = same_name.or_else(|| {
                            let i = old_index.unwrap_or(0).min(self.containers.len().checked_sub(1)?);
                            Some(self.containers[i].id.clone())
                        });
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
        if let Some(focus) = self.term_focus.clone() {
            return self.terminal_keys(ctx, &focus);
        }
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
        // Ctrl+C arrives as a Copy event, so it never triggers the plain `C` (pause) key.
        if ctx.input(|i| i.events.contains(&egui::Event::Copy)) {
            self.copy_id(ctx);
        }
        // Auto-repeat must not count as the confirming press.
        let del = |e: &egui::Event| {
            matches!(e, egui::Event::Key { key: Key::Delete, pressed: true, repeat: false, .. })
        };
        if ctx.input(|i| i.events.iter().any(del)) {
            self.remove();
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.removing.cancel();
        }
        let plain = ctx.input(|i| i.modifiers.is_none());
        if pressed(Key::L) && plain {
            self.console_key(ctx, Tab::Logs);
        }
        if pressed(Key::T) && plain {
            self.console_key(ctx, Tab::Terminal);
        }
        if self.console_open {
            let page = console::ROWS as i32 - 2;
            if pressed(Key::PageUp) {
                self.scroll_console(page);
            }
            if pressed(Key::PageDown) {
                self.scroll_console(-page);
            }
            if pressed(Key::End) {
                self.scroll_console(-1_000_000);
            }
        }
        if pressed(Key::D) {
            self.cycle_scale(ctx);
        }
    }

    /// Repaint cadence: fast only while something animates, else once a second.
    fn next_tick(&self) -> Duration {
        let animating = self.flash.is_some()
            || self.remove_pending()
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
        // Laid out once per frame: a toggle while drawing takes effect on the next one.
        let console = self.console_open;
        let w = canvas_w(console);
        if self.canvas.get_mut().w != w {
            self.canvas = RefCell::new(Canvas::new(w, ui::H));
        }
        let canvas = std::mem::replace(&mut self.canvas, RefCell::new(Canvas::new(0, 0)));
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let origin = ui.max_rect().min;
            let px = Px { canvas: &canvas, ox: 0, oy: 0, origin, s: self.scale as f32 };
            ui::player::draw(self, ui, &px);
            ui::details::draw(self, ui, &px.at(0, ui::PLAYER_H));
            ui::playlist::draw(self, ui, &px.at(0, ui::PLAYER_H + ui::DETAILS_H));
            if console {
                ui::console::draw(self, ui, &px.at(ui::W, 0));
            }

            // One texture upload per frame, scaled with nearest-neighbour for crisp pixels.
            let image = egui::ColorImage {
                size: [w as usize, ui::H as usize],
                pixels: canvas.borrow().pixels.clone(),
            };
            let tex = match &mut self.tex {
                Some(t) => {
                    t.set(image, egui::TextureOptions::NEAREST);
                    t
                }
                None => self.tex.insert(ctx.load_texture("skin", image, egui::TextureOptions::NEAREST)),
            };
            let rect = egui::Rect::from_min_size(origin, window_size(self.scale, console));
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(tex.id(), rect, uv, egui::Color32::WHITE);
        });
        self.canvas = canvas;
        self.sync_logs();
        if console != self.console_open {
            ctx.request_repaint();
        }
        ctx.request_repaint_after(self.next_tick());
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Hang up open shells and let the worker finish doing so.
        self.terms.clear();
        self.cmd = None;
        let _ = self.worker_done.recv_timeout(Duration::from_millis(2500));
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("DockAmp")
            .with_app_id("dockamp")
            .with_inner_size(window_size(DEFAULT_SCALE, false))
            .with_decorations(false)
            .with_resizable(false),
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };
    eframe::run_native("DockAmp", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
