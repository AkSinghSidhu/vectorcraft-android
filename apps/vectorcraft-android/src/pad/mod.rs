//! The Shortcut Pad: a slim strip on the left or right edge of the window with 2 to 10 keys
//! (split above and below) and a wheel, like the ExpressKeys and Touch Ring of a pen tablet.
//! Ported from the photocraft-android shell, adapted to VectorCraft (the wheel drives VectorCraft's
//! view state and freehand tools, and there is no platform menu to hang settings rows on: the
//! gear key at the top of the pad opens the settings).
//!
//! - A key presses its shortcut while it is touched: `Ctrl+Z` undoes, a modifier on its own
//!   (`Alt`) is held, so the pen Alt-clicks while a finger rests on the key.
//! - Pressing the wheel opens a large ring (its size is a setting); sliding the finger around it
//!   zooms, rotates the view or sizes the freehand tools (all smoothly), or steps through
//!   history, at the chosen sensitivity. Tapping the centre switches to the next of the chosen
//!   functions (without opening the ring).
//! - The key at the very bottom, always there, opens the on-screen keyboard (and closes it
//!   again); its letters reach the app's shortcuts.
//! - The gear key at the very top, always there, opens this pad's settings and Touch Input's.

pub mod config;
pub mod router;
pub mod wheel;

use config::{MAX_KEYS, MIN_KEYS, Chord, PadConfig, SENSITIVITY_RANGE, Side, WHEEL_SIZE_RANGE, WheelMode};
use egui::{Align2, Event, FontId, Key, Modifiers, Stroke};
use router::{Control, Env, Layout, PadTouch, Routed, Router, ViewMove};
use vectorcraft_ui_egui::VectorcraftApp;
use vectorcraft_ui_egui::theme::Tokens;

/// The pad's settings, shared between the Shell's settings window and the pad itself.
pub struct Shared {
    pub config: PadConfig,
    path: std::path::PathBuf,
}

impl Shared {
    fn save(&self) {
        if let Err(e) = self.config.save(&self.path) {
            log::error!("couldn't save the Shortcut Pad settings: {e}");
        }
    }
}

pub struct Pad {
    shared: std::rc::Rc<std::cell::RefCell<Shared>>,
    router: Router,
    /// Where the pad was drawn last frame (`None`: hidden).
    layout: Option<Layout>,
    /// The wheel function in use (an index into the configured list).
    mode: usize,
    turn: wheel::Turn,
    /// The ring is being turned.
    turning: bool,
    /// Keys being held, with the chord they pressed (released with the same one even if the
    /// settings change meanwhile).
    held: std::collections::HashMap<usize, Chord>,
    /// Wheel changes waiting for the frame (they need the app and the egui context).
    pending: Vec<wheel::Action>,
    /// The large ring shown while the wheel is pressed.
    big: Option<BigRing>,
    /// Where the finger turning the wheel is.
    finger: Option<egui::Pos2>,
    /// What the wheel press started on, and where; whether the finger has slid since (a tap on
    /// the centre switches function, a slide turns).
    press: Option<(Control, egui::Pos2)>,
    slid: bool,
    /// The keyboard key was tapped (the shell opens or closes the on-screen keyboard).
    keyboard_tapped: bool,
    /// The gear key was tapped (the shell opens the settings window).
    settings_tapped: bool,
    /// The shell has the on-screen keyboard open for the pad (the key is lit).
    pub keyboard_on: bool,
    /// The window's content area last frame (keeps the large ring on screen).
    screen: egui::Rect,
    /// One-shot diagnostics (set when the config was first laid out; remove with the cause).
    logged: bool,
    /// The freehand size as the turn has set it, unrounded, so slow turns add up.
    size_target: Option<f64>,
    /// Canvas gestures to apply to the view (Touch Input).
    views: Vec<ViewMove>,
}

/// The large ring: where it is and how big.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BigRing {
    center: egui::Pos2,
    radius: f32,
}

/// The large ring for a press on the wheel at `wheel`: as near the wheel as it fits on screen
/// (it slides in from the pad's edge onto the canvas).
fn big_ring(wheel: egui::Pos2, radius: f32, screen: egui::Rect, side: Side) -> BigRing {
    let m = radius + 8.0;
    let x = match side {
        Side::Right => wheel.x.min(screen.right() - m),
        Side::Left => wheel.x.max(screen.left() + m),
    };
    let y = if screen.height() > 2.0 * m { wheel.y.clamp(screen.top() + m, screen.bottom() - m) } else { screen.center().y };
    BigRing { center: egui::pos2(x, y), radius }
}

/// The zoom range of the canvas, like the app's own zoom stops (3.13%–640%).
const ZOOM_RANGE: std::ops::RangeInclusive<f64> = 0.0313..=640.0;
/// The freehand tools' size range, like the tool's own option (`tools`).
const SIZE_RANGE: std::ops::RangeInclusive<f64> = 0.1..=1000.0;
/// The freehand tools the wheel sizes (the Pencil's, Paintbrush's, Blob Brush's and Eraser's
/// one `size` option).
const FREEHAND_TOOLS: [&str; 4] = ["pencil", "paintbrush", "blobBrush", "eraser"];

impl Pad {
    pub fn new(files_dir: &std::path::Path) -> Self {
        let path = files_dir.join("shortcut-pad.json");
        let config = PadConfig::load(&path);
        Self {
            shared: std::rc::Rc::new(std::cell::RefCell::new(Shared { config, path })),
            router: Router::default(),
            layout: None,
            mode: 0,
            turn: wheel::Turn::default(),
            turning: false,
            held: std::collections::HashMap::new(),
            pending: Vec::new(),
            big: None,
            finger: None,
            press: None,
            slid: false,
            keyboard_tapped: false,
            settings_tapped: false,
            keyboard_on: false,
            screen: egui::Rect::NOTHING,
            logged: false,
            size_target: None,
            views: Vec::new(),
        }
    }

    /// The config, for the shell to read (Touch Input's settings sit beside the pad's).
    pub fn shared(&self) -> std::rc::Rc<std::cell::RefCell<Shared>> {
        self.shared.clone()
    }

    fn mode(&self) -> WheelMode {
        let s = self.shared.borrow();
        let modes = &s.config.wheel_modes;
        modes.get(self.mode % modes.len().max(1)).copied().unwrap_or(WheelMode::Zoom)
    }

    /// Route a frame's input (from `raw_input_hook`, before modifiers are applied): pad touches
    /// become key presses and wheel commands, canvas gestures move the view and taps press their
    /// shortcut (Touch Input), the rest reaches the app.
    pub fn input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, env: &Env, covered: impl Fn(egui::Pos2) -> bool) {
        let shown = self.shared.borrow().config.enabled;
        let layout = if shown { self.layout.clone() } else { None };
        // A window or menu drawn over the pad last frame (the settings window, a menu dropping
        // down) keeps the touches there.
        let routed = self.router.route(std::mem::take(&mut raw.events), layout.as_ref(), covered, env);
        let mut out = Vec::with_capacity(routed.len());
        if layout.is_none() {
            self.release_all(&mut out);
        }
        for r in routed {
            match r {
                Routed::App(e) => out.push(e),
                Routed::Pad(t) => self.touch(t, &mut out),
                Routed::View(m) => self.views.push(m),
                Routed::Tap(fingers) => {
                    let shortcut = match fingers {
                        2 => &env.config.two_finger_tap,
                        3 => &env.config.three_finger_tap,
                        _ => continue,
                    };
                    match config::parse(shortcut) {
                        Ok(chord) if !chord.is_empty() => {
                            out.extend(press(chord));
                            out.extend(release(chord));
                        }
                        Ok(_) => {}
                        Err(e) => log::warn!("{fingers}-finger tap: {e}"),
                    }
                }
            }
        }
        if self.router.wants_frames() {
            ctx.request_repaint();
        }
        raw.events = out;
    }

    fn touch(&mut self, t: PadTouch, out: &mut Vec<Event>) {
        match t {
            PadTouch::Down(Control::Key(i), _) => {
                let shortcut = self.shared.borrow().config.keys.get(i).map(|k| k.shortcut.clone()).unwrap_or_default();
                match config::parse(&shortcut) {
                    Ok(chord) if !chord.is_empty() => {
                        out.extend(press(chord));
                        if let Some(old) = self.held.insert(i, chord) {
                            out.extend(release(old));
                        }
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("Shortcut Pad key {}: {e}", i + 1),
                }
            }
            PadTouch::Up(Control::Key(i)) => {
                if let Some(chord) = self.held.remove(&i) {
                    out.extend(release(chord));
                }
            }
            PadTouch::Down(Control::Keyboard, _) => self.keyboard_tapped = true,
            PadTouch::Down(Control::Settings, _) => self.settings_tapped = true,
            // The centre is a button: a tap that doesn't slide switches function when lifted,
            // and it neither opens the large ring nor turns.
            PadTouch::Down(Control::Center, p) => {
                self.press = Some((Control::Center, p));
                self.slid = false;
            }
            PadTouch::Move(Control::Center, p) => {
                if self.press.is_some_and(|(_, start)| start.distance(p) > 12.0) {
                    self.slid = true;
                }
            }
            // The ring opens the large ring and turns.
            PadTouch::Down(Control::Ring, p) | PadTouch::Move(Control::Ring, p) => {
                let Some((center, small_r)) = self.layout.as_ref().map(|l| (l.center, l.ring_outer)) else { return };
                if let PadTouch::Down(control, _) = t {
                    self.press = Some((control, p));
                    self.slid = false;
                    self.turn = wheel::Turn::default();
                    self.turning = true;
                    self.size_target = None;
                    let c = &self.shared.borrow().config;
                    self.big = c.enlarge.then(|| big_ring(center, c.wheel_size / 2.0, self.screen, c.side));
                }
                self.finger = Some(p);
                let (c, r) = self.big.map_or((center, small_r), |b| (b.center, b.radius));
                // Too near the centre for a steady angle: wait for the finger to come out.
                let angle = (p.distance(c) >= r * 0.2).then(|| wheel::angle(c, p));
                let (mode, sensitivity) = (self.mode(), self.shared.borrow().config.sensitivity);
                self.pending.extend(self.turn.moved(mode, angle, sensitivity));
            }
            PadTouch::Up(Control::Ring | Control::Center) => {
                if matches!(self.press, Some((Control::Center, _))) && !self.slid {
                    let n = self.shared.borrow().config.wheel_modes.len().max(1);
                    self.mode = (self.mode + 1) % n;
                }
                self.press = None;
                self.turning = false;
                self.big = None;
                self.finger = None;
            }
            _ => {}
        }
    }

    fn release_all(&mut self, out: &mut Vec<Event>) {
        for (_, chord) in self.held.drain() {
            out.extend(release(chord));
        }
        self.turning = false;
        self.big = None;
        self.finger = None;
    }

    /// Apply the wheel's changes (from `logic`, before the app's). A frame's movements are
    /// merged: one zoom, one rotation, one size change, however many touch events came in.
    pub fn run_pending(&mut self, app: &mut VectorcraftApp, ctx: &egui::Context) {
        let views = std::mem::take(&mut self.views);
        if !views.is_empty() {
            crate::touch::apply(app, &views);
            ctx.request_repaint();
        }
        let actions = std::mem::take(&mut self.pending);
        if actions.is_empty() {
            return;
        }
        let (mut zoom, mut rotate, mut size) = (1.0f64, 0.0f64, 1.0f64);
        for a in actions {
            match a {
                wheel::Action::Zoom(f) => zoom *= f as f64,
                wheel::Action::Rotate(d) => rotate += d as f64,
                wheel::Action::Size(f) => size *= f as f64,
                wheel::Action::Command(id) => {
                    // No document to undo or redo: not worth an error for a wheel turn.
                    if let Err(e) = app.run(id, serde_json::json!({})) {
                        log::debug!("Shortcut Pad wheel: {id}: {e}");
                    }
                }
            }
        }
        if zoom != 1.0
            && let Some(v) = app.view_mut()
        {
            v.zoom = (v.zoom * zoom).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
            // The view the document opens with stays superseded.
            v.fitted = true;
        }
        if rotate != 0.0
            && let Some(v) = app.view_mut()
        {
            // As the Rotate View tool drag does: no history step, normalized degrees.
            v.rotation = vectorcraft_geom::normalize_deg(v.rotation + rotate);
            v.fitted = true;
        }
        if size != 1.0 && FREEHAND_TOOLS.contains(&app.session.tool_id())
            && let Err(e) = self.freehand_size(app, size) {
                log::debug!("Shortcut Pad wheel: freehand size: {e}");
            }
        ctx.request_repaint();
    }

    /// The freehand tools' `size`, turned directly (no history: a size is not a document state).
    fn freehand_size(&mut self, app: &mut VectorcraftApp, size: f64) -> Result<(), String> {
        let tool = app.session.tool_id().to_string();
        let options = app.session.tool_options();
        let current = options.get("size").and_then(serde_json::Value::as_f64).unwrap_or(10.0);
        let base = self.size_target.take().unwrap_or(current);
        let target = (base * size).clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end());
        self.size_target = Some(target);
        // A tenth of a point is finer than a finger's jitter and keeps the readout short.
        let rounded = (target * 10.0).round() / 10.0;
        if (rounded - current).abs() < 1e-9 {
            return Ok(());
        }
        app.run("tool.setOption", serde_json::json!({ "tool": tool, "values": { "size": rounded } })).map(|_| ())
    }

    /// What the large ring shows for the current function.
    fn readout(app: &VectorcraftApp, mode: WheelMode) -> String {
        let view = app.view();
        match mode {
            WheelMode::Zoom => view.map(|v| format!("{:.0}%", v.zoom * 100.0)).unwrap_or_default(),
            WheelMode::Rotate => view.map(|v| format!("{:.0}°", v.rotation)).unwrap_or_default(),
            WheelMode::Freehand => {
                if FREEHAND_TOOLS.contains(&app.session.tool_id()) {
                    let size = app.session.tool_options().get("size").and_then(serde_json::Value::as_f64).unwrap_or(10.0);
                    format!("{size:.1} pt")
                } else {
                    "—".into()
                }
            }
            WheelMode::History => "Undo ◀ ▶ Redo".into(),
        }
    }

    fn paint_big(&self, p: &egui::Painter, b: BigRing, mode: WheelMode, readout: &str, tk: &Tokens) {
        let (c, r) = (b.center, b.radius);
        let band = r * 0.32;
        let mid = r - band / 2.0;
        p.circle_filled(c, r + 2.0, tk.error);
        p.circle_filled(c, r, tk.panel_darker);
        p.circle_stroke(c, mid, Stroke::new(band, tk.panel));
        p.circle_stroke(c, r, Stroke::new(2.0, tk.accent));
        for k in 0..24 {
            let a = (k as f32 * wheel::TICK).to_radians();
            p.circle_filled(c + egui::vec2(a.cos(), a.sin()) * mid, 1.6, tk.text_disabled);
        }
        if let Some(f) = self.finger
            && f.distance(c) > 1.0
        {
            let dir = (f - c).normalized();
            p.circle_filled(c + dir * mid, band * 0.38, tk.accent);
        }
        p.text(c - egui::vec2(0.0, 10.0), Align2::CENTER_CENTER, mode.short(), FontId::proportional(15.0), tk.text_dim);
        p.text(c + egui::vec2(0.0, 12.0), Align2::CENTER_CENTER, readout, FontId::proportional(18.0), tk.text);
    }

    /// Draw the pad (before the app, which then lays out in the rest of the window).
    pub fn show(&mut self, ui: &mut egui::Ui, tk: &Tokens, app: &VectorcraftApp) {
        self.screen = ui.ctx().content_rect();
        let config = self.shared.borrow().config.clone();
        if !config.enabled {
            self.layout = None;
            return;
        }
        let panel = match config.side {
            Side::Left => egui::Panel::left("shortcut_pad"),
            Side::Right => egui::Panel::right("shortcut_pad"),
        };
        let frame = egui::Frame::new().fill(tk.panel).stroke(Stroke::new(1.0, tk.divider));
        let mode = self.mode();
        let layout = panel
            .exact_size(router::WIDTH)
            .resizable(false)
            .frame(frame)
            .show(ui, |ui| {
                let l = Layout::new(ui.max_rect(), config.keys.len());
                self.paint(ui.painter(), &l, &config, mode, tk);
                l
            })
            .inner;
        self.layout = Some(layout.clone());
        if !self.logged && std::env::var("NO_LOG").is_err() {
            self.logged = true;
            log::info!(
                "pad: frame {:?} keys {} (first {:?}) center ({:.1},{:.1}) ring {:.1} settings {:?} keyboard {:?}; colors input {:?} panel {:?}",
                ui.max_rect(),
                config.keys.len(),
                layout.keys.first(),
                layout.center.x,
                layout.center.y,
                layout.ring_outer,
                layout.settings,
                layout.keyboard,
                tk.input,
                tk.panel
            );
        }
        if let Some(b) = self.big {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("shortcut_pad_ring")));
            self.paint_big(&painter, b, mode, &Self::readout(app, mode), tk);
        }
    }

    fn paint(&self, p: &egui::Painter, l: &Layout, config: &PadConfig, mode: WheelMode, tk: &Tokens) {
        // Labels a size smaller on keys shrunk to fit a short screen.
        let font = FontId::proportional(if l.key_height() < 36.0 { 11.0 } else { 12.0 });
        for (i, rect) in l.keys.iter().enumerate() {
            let down = self.held.contains_key(&i);
            p.rect(*rect, tk.radius, if down { tk.accent } else { tk.input }, Stroke::new(1.0, tk.input_border), egui::StrokeKind::Inside);
            let label = config.keys.get(i).map(|k| k.label.as_str()).unwrap_or("");
            let label = label.to_string();
            let color = if down { tk.text_strong } else { tk.text };
            // The largest size that fits the key's width (typed labels can be wide), clipped
            // to the key at the smallest.
            let room = rect.width() - 6.0;
            let galley = [font.size, font.size - 1.0, font.size - 2.0, 9.0]
                .into_iter()
                .map(|size| p.layout_no_wrap(label.clone(), FontId::proportional(size), color))
                .find(|g| g.size().x <= room)
                .unwrap_or_else(|| p.layout_no_wrap(label.clone(), FontId::proportional(9.0), color));
            p.with_clip_rect(rect.shrink(2.0)).galley(rect.center() - galley.size() / 2.0, galley, color);
        }
        // The ring, with click-stop marks, and the centre button naming the function.
        let ring = if self.turning { tk.accent } else { tk.input_border };
        p.circle(l.center, l.ring_outer, tk.input, Stroke::new(1.5, ring));
        let mid = (l.ring_outer + l.ring_inner) / 2.0;
        for k in 0..24 {
            let a = (k as f32 * wheel::TICK).to_radians();
            p.circle_filled(l.center + egui::vec2(a.cos(), a.sin()) * mid, 1.2, tk.text_disabled);
        }
        p.circle(l.center, l.ring_inner, tk.panel_darker, Stroke::new(1.0, tk.input_border));
        let n = config.wheel_modes.len();
        let current = self.mode % n.max(1);
        // One light per function, the current one lit (like a tablet's ring LEDs).
        for k in 0..n {
            let a = (-90.0 + (k as f32 - (n as f32 - 1.0) / 2.0) * 16.0f32).to_radians();
            let at = l.center + egui::vec2(a.cos(), a.sin()) * (l.ring_outer - 5.0);
            p.circle_filled(at, 2.2, if k == current { tk.accent } else { tk.text_disabled });
        }
        p.text(l.center, Align2::CENTER_CENTER, mode.short(), FontId::proportional(9.0), tk.text_dim);
        self.paint_gear_key(p, l.settings, tk);
        self.paint_keyboard_key(p, l.keyboard, tk);
    }

    /// The settings key: a gear drawn in it, lit while the settings window is open. It is a
    /// rounded button like the other keys; the children read it in the shell.
    fn paint_gear_key(&self, p: &egui::Painter, rect: egui::Rect, tk: &Tokens) {
        p.rect(rect, tk.radius, tk.input, Stroke::new(1.0, tk.input_border), egui::StrokeKind::Inside);
        let c = rect.center();
        let r = rect.height() * 0.22;
        let ink = tk.icon;
        p.circle_filled(c, r, ink);
        // Teeth: short spokes all round.
        for k in 0..8 {
            let a = (k as f32 * 45.0f32).to_radians();
            let dir = egui::vec2(a.cos(), a.sin());
            p.line_segment([c + dir * r, c + dir * (r + 2.5)], Stroke::new(1.6, ink));
        }
        p.circle_filled(c, r * 0.4, tk.input);
    }

    /// The keyboard key: a small keyboard drawn in it (keys and a space bar), lit while open.
    fn paint_keyboard_key(&self, p: &egui::Painter, rect: egui::Rect, tk: &Tokens) {
        let on = self.keyboard_on;
        p.rect(rect, tk.radius, if on { tk.accent } else { tk.input }, Stroke::new(1.0, tk.input_border), egui::StrokeKind::Inside);
        let ink = if on { tk.text_strong } else { tk.text };
        let body = egui::Rect::from_center_size(rect.center(), egui::vec2(30.0, 18.0).min(rect.size() - egui::vec2(6.0, 6.0)));
        p.rect_stroke(body, 2.0, Stroke::new(1.2, ink), egui::StrokeKind::Inside);
        let (cols, key) = (5, body.width() / 7.0);
        for row in 0..2 {
            for col in 0..cols {
                let x = body.left() + key * (1.0 + col as f32 * 1.0) + key / 2.0;
                let y = body.top() + body.height() * (0.3 + row as f32 * 0.22);
                p.circle_filled(egui::pos2(x, y), 1.2, ink);
            }
        }
        let bar = body.bottom() - body.height() * 0.24;
        p.line_segment([egui::pos2(body.center().x - key * 1.5, bar), egui::pos2(body.center().x + key * 1.5, bar)], Stroke::new(1.5, ink));
    }

    /// The settings body the shell shows in its window: the pad's own settings, the Touch Input
    /// section beside them (the shell draws the Touch Input rows after this). Saved as it is
    /// changed, like the old menu rows did.
    pub fn settings_body(&mut self, ui: &mut egui::Ui, tk: &Tokens) {
        let before = self.shared.borrow().config.clone();
        let mut c = before.clone();
        ui.strong("Shortcut Pad");
        ui.checkbox(&mut c.enabled, "Show the Shortcut Pad");
        ui.horizontal(|ui| {
            ui.label("Side:");
            ui.radio_value(&mut c.side, Side::Left, "Left");
            ui.radio_value(&mut c.side, Side::Right, "Right");
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Number of keys:");
            let mut n = c.keys.len();
            if ui.add(egui::Slider::new(&mut n, MIN_KEYS..=MAX_KEYS)).changed() {
                c.set_key_count(n);
            }
            ui.label(format!("{} above the wheel, {} below", n.div_ceil(2), n / 2));
        });
        ui.label("Keys (top to bottom). A modifier on its own, such as Alt, is held while the key is touched.");
        let above = c.keys.len().div_ceil(2);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            egui::Grid::new("pad_keys").num_columns(4).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("");
                ui.label("Label");
                ui.label("Preset");
                ui.label("Shortcut");
                ui.end_row();
                for (i, key) in c.keys.iter_mut().enumerate() {
                    ui.label(format!("Key {}", i + 1)).on_hover_text(if i < above { "Above the wheel" } else { "Below the wheel" });
                    ui.add_sized([84.0, 24.0], egui::TextEdit::singleline(&mut key.label).char_limit(8));
                    egui::ComboBox::from_id_salt(("pad_preset", i)).selected_text("Choose…").width(110.0).show_ui(ui, |ui| {
                        for (name, label, shortcut) in config::PRESETS {
                            if ui.selectable_label(key.shortcut == *shortcut, format!("{name}  ({shortcut})")).clicked() {
                                key.label = (*label).into();
                                key.shortcut = (*shortcut).into();
                            }
                        }
                        if ui.selectable_label(key.shortcut.is_empty(), "Nothing").clicked() {
                            key.label.clear();
                            key.shortcut.clear();
                        }
                    });
                    ui.vertical(|ui| {
                        ui.add_sized([130.0, 24.0], egui::TextEdit::singleline(&mut key.shortcut).hint_text("e.g. Ctrl+Z"));
                        if let Err(e) = config::parse(&key.shortcut) {
                            ui.colored_label(tk.error, e);
                        }
                    });
                    ui.end_row();
                }
            });
        });
        ui.separator();
        ui.label("Wheel: tapping its centre switches between");
        for m in WheelMode::ALL {
            let mut on = c.wheel_modes.contains(&m);
            let last = on && c.wheel_modes.len() == 1;
            if ui.add_enabled(!last, egui::Checkbox::new(&mut on, m.label())).changed() {
                if on {
                    // Keep the configured order: the order of ALL.
                    c.wheel_modes.push(m);
                    c.wheel_modes.sort_by_key(|x| WheelMode::ALL.iter().position(|a| a == x));
                } else {
                    c.wheel_modes.retain(|x| *x != m);
                }
            }
        }
        ui.checkbox(&mut c.enlarge, "Open a large ring when the wheel is pressed");
        ui.add_enabled(c.enlarge, egui::Slider::new(&mut c.wheel_size, WHEEL_SIZE_RANGE).text("Ring size").suffix(" pt").integer());
        ui.add(egui::Slider::new(&mut c.sensitivity, SENSITIVITY_RANGE).logarithmic(true).text("Sensitivity").suffix("×"))
            .on_hover_text("At 1×, a quarter turn doubles the zoom, a third of a turn doubles the freehand size");
        ui.separator();
        if ui.button("Restore Pad Defaults").clicked() {
            c = PadConfig::default();
        }
        if c != before {
            self.shared.borrow_mut().config = c.sanitized();
            self.shared.borrow().save();
        }
    }

    /// Whether the keyboard key was tapped since the last call.
    pub fn take_keyboard_tap(&mut self) -> bool {
        std::mem::take(&mut self.keyboard_tapped)
    }

    /// Whether the gear key was tapped since the last call.
    pub fn take_settings_tap(&mut self) -> bool {
        std::mem::take(&mut self.settings_tapped)
    }

    /// Whether the pad needs frames: a finger is on it.
    pub fn active(&self) -> bool {
        self.router.pad_active()
    }
}

fn key_event(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE }
}

fn modifier_keys(c: Chord) -> Vec<Key> {
    [(c.ctrl, Key::ControlLeft), (c.alt, Key::AltLeft), (c.shift, Key::ShiftLeft)].into_iter().filter(|(on, _)| *on).map(|(_, k)| k).collect()
}

/// The key events of pressing a chord: its modifiers, then its key.
pub fn press(c: Chord) -> Vec<Event> {
    let mut v: Vec<Event> = modifier_keys(c).into_iter().map(|k| key_event(k, true)).collect();
    v.extend(c.key.map(|k| key_event(k, true)));
    v
}

/// The key events of releasing a chord: its key, then its modifiers.
pub fn release(c: Chord) -> Vec<Event> {
    let mut v: Vec<Event> = c.key.map(|k| key_event(k, false)).into_iter().collect();
    v.extend(modifier_keys(c).into_iter().rev().map(|k| key_event(k, false)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::touch::config::TouchConfig;
    use egui::{Rect, TouchDeviceId, TouchId, TouchPhase, pos2, vec2};

    fn pad() -> (Pad, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("vectorcraft-padmod-{}-{}", std::process::id(), rand()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = Pad::new(&dir);
        p.layout = Some(Layout::new(Rect::from_min_size(pos2(1000.0, 0.0), vec2(router::WIDTH, 800.0)), 4));
        (p, dir)
    }

    fn rand() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    fn touch_ev(id: u64, phase: TouchPhase, pos: egui::Pos2) -> Event {
        Event::Touch { device_id: TouchDeviceId(1), id: TouchId(id), phase, pos, force: None }
    }

    fn run(p: &mut Pad, events: Vec<Event>) -> Vec<Event> {
        let mut raw = egui::RawInput { events, ..Default::default() };
        let config = TouchConfig::default();
        let env = Env { canvas: None, is_finger: &|_| true, pen_near: false, config: &config, list_item: &|_| false, now: 0.0 };
        p.input(&egui::Context::default(), &mut raw, &env, |_| false);
        raw.events
    }

    fn keys(events: &[Event]) -> Vec<(Key, bool)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Key { key, pressed, .. } => Some((*key, *pressed)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_key_presses_its_shortcut_and_releases_it() {
        let (mut p, dir) = pad();
        let k0 = p.layout.clone().unwrap().keys[0].center();
        // Default key 1: Undo (Ctrl+Z).
        let down = run(&mut p, vec![touch_ev(1, TouchPhase::Start, k0)]);
        assert_eq!(keys(&down), [(Key::ControlLeft, true), (Key::Z, true)]);
        assert!(!down.iter().any(|e| matches!(e, Event::Touch { .. } | Event::PointerButton { .. })), "the finger doesn't reach the canvas");
        let up = run(&mut p, vec![touch_ev(1, TouchPhase::End, k0)]);
        assert_eq!(keys(&up), [(Key::Z, false), (Key::ControlLeft, false)]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_modifier_key_is_held_while_touched() {
        let (mut p, dir) = pad();
        let k2 = p.layout.clone().unwrap().keys[2].center(); // default: Alt
        assert_eq!(keys(&run(&mut p, vec![touch_ev(1, TouchPhase::Start, k2)])), [(Key::AltLeft, true)]);
        assert!(keys(&run(&mut p, vec![touch_ev(1, TouchPhase::Move, k2)])).is_empty(), "held, not repeated");
        // Hiding the pad releases it (no stuck Alt).
        p.shared.borrow_mut().config.enabled = false;
        assert_eq!(keys(&run(&mut p, vec![])), [(Key::AltLeft, false)]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_wheel_queues_changes_and_the_centre_switches_function() {
        let (mut p, dir) = pad();
        p.screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1072.0, 800.0));
        let l = p.layout.clone().unwrap();
        let at = |deg: f32| l.center + egui::vec2(deg.to_radians().cos(), deg.to_radians().sin()) * (l.ring_outer - 4.0);
        run(&mut p, vec![touch_ev(1, TouchPhase::Start, at(90.0))]);
        let big = p.big.unwrap_or_else(|| panic!("pressing the wheel opens the large ring"));
        assert!(big.center.x + big.radius <= p.screen.right(), "on screen: {big:?}");
        let on_big = |deg: f32| big.center + egui::vec2(deg.to_radians().cos(), deg.to_radians().sin()) * big.radius * 0.8;
        run(&mut p, vec![touch_ev(1, TouchPhase::Move, on_big(0.0)), touch_ev(1, TouchPhase::Move, on_big(45.0))]);
        assert!(p.pending.iter().any(|a| matches!(a, wheel::Action::Zoom(f) if *f > 1.0)), "clockwise zooms in: {:?}", p.pending);
        run(&mut p, vec![touch_ev(1, TouchPhase::End, on_big(45.0))]);
        assert!(p.big.is_none(), "lifting closes it");
        p.pending.clear();
        // Tap the centre: Rotate View, and no large ring.
        run(&mut p, vec![touch_ev(2, TouchPhase::Start, l.center)]);
        assert!(p.big.is_none() && !p.turning, "the centre button doesn't open the large ring");
        run(&mut p, vec![touch_ev(2, TouchPhase::End, l.center)]);
        assert_eq!(p.mode(), WheelMode::Rotate);
        assert!(p.pending.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_keyboard_and_gear_keys_ask_the_shell() {
        let (mut p, dir) = pad();
        let (kb, gear) = {
            let l = p.layout.clone().unwrap();
            (l.keyboard.center(), l.settings.center())
        };
        assert!(!p.take_keyboard_tap() && !p.take_settings_tap());
        let out = run(&mut p, vec![touch_ev(1, TouchPhase::Start, gear), touch_ev(1, TouchPhase::End, gear)]);
        assert!(keys(&out).is_empty() && !out.iter().any(|e| matches!(e, Event::Touch { .. })));
        assert!(p.take_settings_tap() && !p.take_settings_tap(), "once per tap");
        let out = run(&mut p, vec![touch_ev(1, TouchPhase::Start, kb), touch_ev(1, TouchPhase::End, kb)]);
        assert!(keys(&out).is_empty());
        assert!(p.take_keyboard_tap());
        assert!(!p.take_keyboard_tap(), "once per tap");
        std::fs::remove_dir_all(dir).ok();
    }

    /// Ten keys: the sixth (the first below the wheel) presses its own shortcut.
    #[test]
    fn a_ten_key_pad_routes_every_key() {
        let (mut p, dir) = pad();
        p.shared.borrow_mut().config.set_key_count(10);
        let count = p.shared.borrow().config.keys.len();
        p.layout = Some(Layout::new(Rect::from_min_size(pos2(1000.0, 0.0), vec2(router::WIDTH, 800.0)), count));
        let l = p.layout.clone().unwrap();
        assert!(l.keys[5].top() > l.center.y, "key 6 is the first below the wheel");
        let shortcut = p.shared.borrow().config.keys[5].shortcut.clone();
        let chord = config::parse(&shortcut).unwrap();
        let down = run(&mut p, vec![touch_ev(1, TouchPhase::Start, l.keys[5].center())]);
        assert_eq!(keys(&down), keys(&press(chord)), "{shortcut}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn press_and_release_mirror_each_other() {
        let c = config::parse("Ctrl+Shift+Alt+S").unwrap();
        let down: Vec<(Key, bool)> = keys(&press(c));
        let up: Vec<(Key, bool)> = keys(&release(c));
        assert_eq!(down.iter().map(|k| k.0).collect::<Vec<_>>(), up.iter().rev().map(|k| k.0).collect::<Vec<_>>());
    }

    /// The pad draws where it shows: in a frame like the shell's, every key and the wheel is
    /// painted inside the strip the panel reserves (right edge), not somewhere invisible.
    #[test]
    fn show_paints_the_keys_and_wheel_in_its_strip() {
        let (mut p, dir) = pad();
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1072.0, 800.0))),
            time: Some(0.0),
            ..Default::default()
        };
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let mut out = ctx.run_ui(raw, |ui| {
            let tk = Tokens::get(ui.ctx());
            p.show(ui, &tk, &app);
        });
        // The painted shapes' clips and rects; anything not a shape in the strip's x range is
        // elsewhere (the strip is 1072−72=1000…1072).
        let in_strip = |min: f32, max: f32| (1000.0..=1072.0).contains(&min) && (1000.0..=1072.0).contains(&max);
        let mut rect_shapes = 0;
        let mut strip_hits = 0;
        let mut text_shapes = 0;
        let mut text_positions: Vec<(f32, f32)> = Vec::new();
        for s in &out.shapes {
            match &s.shape {
                egui::Shape::Rect(r) => {
                    rect_shapes += 1;
                    let (x0, x1) = (r.rect.min.x, r.rect.max.x);
                    if in_strip(x0, x1) {
                        strip_hits += 1;
                    }
                }
                egui::Shape::Text(t) => {
                    text_shapes += 1;
                    text_positions.push((t.pos.x, t.pos.y));
                }
                _ => {}
            }
        }
        // The keys and wheel: many rects in the strip (4 keys ×2 (fill+stroke), wheel circle
        // fill+2 strokes, 24 ring marks, 4 lights, centre label).
        assert!(strip_hits >= 6, "only {strip_hits} of the {rect_shapes} rects in the pad's strip; text at {text_positions:?}");
        assert!(text_shapes >= 5, "key labels and the wheel's mode label are text; {text_shapes} text shapes");
        // The frame's font raster isn't used by this test.
        out.textures_delta.clear();
        std::fs::remove_dir_all(dir).ok();
    }
}
