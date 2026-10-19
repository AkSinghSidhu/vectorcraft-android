//! The Android entry point: GameActivity starts `android_main` on its own thread once the
//! library is loaded. The [`Shell`] runs [`VectorcraftApp`] and adds what the system doesn't
//! hand egui by itself: the on-screen keyboard's text ([`crate::ime`]) and hardware keyboard
//! modifiers ([`crate::keys`]).

use crate::bridge::Bridge;
use crate::touch::config::{PenAction, PenControl};
use std::path::PathBuf;
use std::sync::Arc;
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{UiState, VectorcraftApp};
use winit::platform::android::activity::AndroidApp;
use winit::platform::android::activity::input::{ImeOptions, InputType, TextInputAction, TextInputState, TextSpan};

/// android-activity looks the entry point up by name, hence `no_mangle` (this crate's only
/// `unsafe`, allowed on this one function).
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("VectorCraft"));
    // Panics end up in logcat rather than vanishing with the process.
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    if let Err(e) = run(app) {
        log::error!("VectorCraft stopped: {e}");
    }
}

/// Where the app's own files live and what belongs under them (preferences, libraries, document
/// copies the Activity manages through the bridge).
fn run(android: AndroidApp) -> Result<(), String> {
    let files_dir = android.internal_data_path().unwrap_or_else(crate::services::fallback_files_dir);
    log::info!("VectorCraft {} starting; data in {}", env!("CARGO_PKG_VERSION"), files_dir.display());
    let bridge = Arc::new(Bridge::new(&files_dir).map_err(|e| format!("couldn't create the file bridge: {e}"))?);

    let mut options = eframe::NativeOptions { renderer: eframe::Renderer::Wgpu, ..Default::default() };
    // eframe hands it to winit's event loop (`with_android_app`).
    options.android_app = Some(android.clone());
    // One frame queued, not two: the canvas is rasterized on the CPU and the GPU only composites
    // it, so the window answers the pointer a frame sooner (#444).
    options.wgpu_options.surface = eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY;
    // Vulkan first, GLES 3 where a device has no usable Vulkan driver.
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN | eframe::wgpu::Backends::GL;
        // The window's surface may be as large as the adapter allows, not eframe's fixed 8192 px:
        // a screen stretched wider than that otherwise fails to configure its surface and the app
        // closes (#1459).
        let device = create.device_descriptor.clone();
        create.device_descriptor = std::sync::Arc::new(move |adapter: &eframe::wgpu::Adapter| {
            let mut descriptor = device(adapter);
            descriptor.required_limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d;
            descriptor
        });
    }

    // The saved preferences come back as on the desktop (written out after each confirmed change).
    let saved: Option<UiState> = std::fs::read(files_dir.join("prefs.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());

    eframe::run_native(
        "VectorCraft",
        options,
        Box::new(move |cc| {
            let mut app = VectorcraftApp::new(Session::new(), crate::services::services(bridge.clone()));
            // Fonts installed into the app (the engine's text.addFontFiles copies into the app's
            // own Fonts folder) load with the system's on every start, the way the desktop keeps
            // that folder next to its settings.
            vectorcraft_text::set_app_font_dir(fonts_dir(&files_dir));
            load_prefs(&mut app, saved);
            // ICC profiles, plug-ins and PDF presets live in their folders under the app's data.
            app.session.attach_profile_store(dir(&files_dir, "Color Profiles"));
            app.session.attach_plugin_store(dir(&files_dir, "Plug-ins"));
            app.session.attach_pdf_preset_store(dir(&files_dir, "PDF Presets"));
            // The User Defined swatch and graphic style libraries and the Libraries panel's
            // libraries live in their folders; the persistent panels reopen from these.
            app.session.swatch_libraries.set_user_dir(Some(files_dir.join("Swatches").to_string_lossy().to_string()));
            app.session.style_libraries.set_user_dir(Some(files_dir.join("Graphic Styles").to_string_lossy().to_string()));
            app.session.libraries.set_dir(Some(files_dir.join("Libraries").to_string_lossy().to_string()));
            // Data Recovery copies (a save that lost its provider lands there too) sit next to
            // the app's data, not somewhere Android chooses.
            app.session.recovery.set_default_folder(Some(files_dir.join("Data Recovery").to_string_lossy().to_string()));

            // The Shortcut Pad (keys, wheel and the touch router) and Touch Input's settings:
            // configured in the app's own files directory, like the desktop's preferences.
            let pad = crate::pad::Pad::new(&files_dir);
            let touch = crate::touch::TouchInput::new(&files_dir);

            let graphics_loss = vectorcraft_ui_egui::graphics::GraphicsLoss::default();
            if let Some(rs) = &cc.wgpu_render_state {
                let info = rs.adapter.get_info();
                let summary = adapter_summary(&info);
                log::info!("rendering with {summary}");
                app.graphics_adapter = Some(summary.clone());
                let (loss, ctx) = (graphics_loss.clone(), cc.egui_ctx.clone());
                rs.device.set_device_lost_callback(move |reason, msg| loss.report(&ctx, format!("{reason:?}: {msg}")));
            }
            // Documents other apps sent (Open with / Share to) arrive while the app runs; a
            // background thread moves them into its two inboxes and wakes the UI.
            let inbox = app.services.inbox.clone().unwrap_or_else(|| Arc::default());
            let place = app.services.place_inbox.clone().unwrap_or_else(|| Arc::default());
            crate::services::watch_incoming(bridge.clone(), inbox, place, cc.egui_ctx.clone());

            // The Activity's pen hook feeds every stylus sample here: the first is logged once,
            // and while a control set to erase is in use the flag below stands (the Eraser tool
            // switch in `raw_input_hook`). The pointer's pressure already flows through winit
            // itself (`Touch.force`); the whole path is spliced back per frame (see
            // `pen_path`).
            let first = std::sync::atomic::AtomicBool::new(true);
            let erasing = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let sink = {
                let erasing = erasing.clone();
                move |sample: Option<vectorcraft_tablet::Sample>| {
                    erasing.store(sample.is_some_and(|s| s.eraser), std::sync::atomic::Ordering::Relaxed);
                    if first.swap(false, std::sync::atomic::Ordering::Relaxed)
                        && let Some(s) = sample
                    {
                        log::info!("first pen sample: {s:?}");
                    }
                }
            };
            if let Err(e) = vectorcraft_tablet::android::install(sink) {
                log::warn!("pen hook: {e}");
            }

            Ok(Box::new(Shell {
                app,
                android,
                bridge,
                files_dir: files_dir.clone(),
                ime: String::new(),
                editing: false,
                soft_keyboard: false,
                keys: Default::default(),
                pen_seen: false,
                quitting: false,
                pad,
                touch,
                drawn: Default::default(),
                settings_open: false,
                pen_path: Vec::new(),
                pen_erasing: erasing,
                erasing_tool: None,
            }))
        }),
    )
    .map_err(|e| e.to_string())
}

/// The fonts folder under the app's data: fonts installed there load on every start.
pub fn fonts_dir(files_dir: &PathBuf) -> PathBuf {
    dir(files_dir, "Fonts")
}

fn dir(files_dir: &PathBuf, name: &str) -> PathBuf {
    PathBuf::from(files_dir).join(name)
}

/// The saved preferences come back, as on the desktop: the app's UI state (sanitized) and then
/// everything the Preferences dialog keeps out of it.
fn load_prefs(app: &mut VectorcraftApp, saved: Option<UiState>) {
    if let Some(ui) = saved {
        app.ui = ui.sanitized();
    }
    vectorcraft_ui_egui::prefs_dialog::restore(app);
}

/// "name (backend, kind)" of the adapter the window renders with, for Help › About and reports.
fn adapter_summary(info: &eframe::wgpu::AdapterInfo) -> String {
    format!("{} ({:?}, {:?})", info.name.trim(), info.backend, info.device_type)
}

/// The app plus what the Android shell adds each frame: on-screen keyboard text (see
/// [`crate::ime`]) and hardware keyboard modifiers ([`crate::keys`]).
struct Shell {
    app: VectorcraftApp,
    android: AndroidApp,
    bridge: Arc<Bridge>,
    files_dir: PathBuf,
    /// The keyboard's text as last seen.
    ime: String,
    /// A text field had focus last frame.
    editing: bool,
    /// The on-screen keyboard is deliberately up for shortcuts too (its hide key and Back put
    /// it away; winit drops those input events, so the Activity leaves a marker file instead).
    soft_keyboard: bool,
    /// Hardware keyboard modifiers (see [`crate::keys`]).
    keys: crate::keys::Held,
    /// A pen has been seen in this session: fingers only navigate from then on (see
    /// `VectorcraftApp`'s touch gestures; set once, on the UI thread, from the Activity's feed).
    pen_seen: bool,
    /// File › Exit was asked for: the Activity finishes (and ends the process) at its next poll
    /// (~150 ms). The UI keeps drawing until then; asking again would log a duplicate.
    quitting: bool,
    /// The Shortcut Pad: keys, wheel and the window's touch router (see [`crate::pad`]).
    pad: crate::pad::Pad,
    /// Touch gestures and their settings (see [`crate::touch`]).
    touch: crate::touch::TouchInput,
    /// Where the last frame drew everything (see [`crate::touch::Snapshot`]).
    drawn: crate::touch::Snapshot,
    /// The Shortcut Pad's and Touch Input's settings window (the pad's top gear key opens it).
    settings_open: bool,
    /// The pen's samples of this frame not yet spliced into the input (see [`crate::pen_path`]).
    pen_path: Vec<(f32, f32)>,
    /// Set by the pen hook while one of the controls set to erase is in use; the Eraser tool is
    /// selected for it and the previous tool comes back after ([`Self::erasing_tool`]).
    pen_erasing: Arc<std::sync::atomic::AtomicBool>,
    /// The tool to put back when the pen stops erasing.
    erasing_tool: Option<&'static str>,
}

impl Shell {
    /// Set the keyboard up for a text field or for shortcuts (`flags` on top of plain text) and
    /// empty its buffer.
    fn reset_keyboard(&mut self, flags: InputType) {
        self.android.set_ime_editor_info(InputType::TYPE_CLASS_TEXT | flags, TextInputAction::None, ImeOptions::IME_FLAG_NO_FULLSCREEN);
        self.android.set_text_input_state(TextInputState { text: String::new(), selection: TextSpan { start: 0, end: 0 }, compose_region: None });
        self.ime.clear();
    }

    /// Write the preferences, the way the desktop does between its saves: the app's own state
    /// plus the engine's (Save As names, recent files and the rest, whose `to_json`).
    fn save_prefs(&mut self) {
        let mut ui = self.app.ui.clone();
        ui.engine_prefs = self.app.session.prefs.to_json();
        let bytes = match serde_json::to_vec_pretty(&ui) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("preferences couldn't be serialized: {e}");
                return;
            }
        };
        let p = self.files_dir.join("prefs.json");
        if let Err(e) = vectorcraft_engine::cmd::fileio::write_atomic(&p, &bytes) {
            log::warn!("couldn't write the preferences: {e}");
        }
    }
}

impl eframe::App for Shell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The pen's presence is state the Activity owns (thread-safe); here on the UI thread it
        // becomes the app's driver: once a pen has been seen, fingers only navigate (palm
        // rejection, vectorcraft's touch gestures).
        if !self.pen_seen && vectorcraft_tablet::android::pen_seen_within(std::time::Duration::from_secs(60)) {
            self.pen_seen = true;
            vectorcraft_ui_egui::touch::set_source(vectorcraft_ui_egui::touch::Source::Pen);
            log::info!("a pen is in use: the pointer is driven by it");
        }
        // Wheel turns and gestures move the view before the app's own logic reads it.
        self.pad.run_pending(&mut self.app, ctx);
        self.app.logic(ctx);
        // File › Exit on Android asks the Activity to finish through the bridge (winit's event
        // loop can't be created twice in one process, so the next launch must start a new one;
        // see `VectorCraftActivity.onDestroy`). Preferences first: Exit is a confirmed save too.
        if self.app.ui.status == "quit" && !self.quitting {
            self.quitting = true;
            self.save_prefs();
            if let Err(e) = self.bridge.request_quit() {
                log::error!("couldn't ask the Activity to finish: {e}");
            }
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // The pad's keyboard key opens or closes the on-screen keyboard (it stays open, typing
        // shortcuts, until the key is tapped again).
        if self.pad.take_keyboard_tap() {
            self.soft_keyboard = !self.soft_keyboard;
            if self.soft_keyboard {
                self.bridge.take_keyboard_hidden();
                if !self.editing {
                    self.reset_keyboard(InputType::TYPE_TEXT_FLAG_NO_SUGGESTIONS | InputType::TYPE_TEXT_VARIATION_VISIBLE_PASSWORD);
                }
                // Explicitly (not "implicit"), so Android shows it with no text view focused.
                self.android.show_soft_input(false);
            } else {
                self.android.hide_soft_input(false);
            }
            self.pad.keyboard_on = self.soft_keyboard;
        }
        if self.soft_keyboard && self.bridge.take_keyboard_hidden() {
            // Put away with its own hide key or Back (it covers the pad's key while open).
            self.soft_keyboard = false;
            self.pad.keyboard_on = false;
        }
        let editing = ctx.egui_wants_keyboard_input();
        if editing && !self.editing {
            // A field just gained focus. Multi-line, so the keyboard's Enter key types a newline
            // into the text state (an editor action would be dropped by winit), which
            // `ime::diff` turns into Enter.
            self.reset_keyboard(InputType::TYPE_TEXT_FLAG_MULTI_LINE);
        }
        if !editing && self.editing {
            if self.soft_keyboard {
                // Back to typing shortcuts.
                self.reset_keyboard(InputType::TYPE_TEXT_FLAG_NO_SUGGESTIONS);
            } else {
                // Focus left the field (Enter, a tap elsewhere): put the keyboard away, which
                // winit doesn't, so it stops covering the dialog's buttons.
                self.android.hide_soft_input(false);
            }
        }
        self.editing = editing;
        if editing || self.soft_keyboard {
            let now = self.android.text_input_state().text;
            if now != self.ime {
                let edit = crate::ime::diff(&self.ime, &now);
                if editing {
                    raw.events.extend(crate::ime::events(&edit));
                    self.ime = now;
                } else {
                    // No field to type into: the letters are shortcuts. Each is handled once,
                    // from an empty buffer again (no suggestions, so nothing is composing).
                    raw.events.extend(crate::ime::key_events(&edit));
                    self.ime = now;
                    if self.soft_keyboard {
                        self.reset_keyboard(InputType::TYPE_TEXT_FLAG_NO_SUGGESTIONS);
                    }
                }
            }
            // GameActivity's text events don't wake the UI (winit drops them): poll while typing.
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // The pad's touches become key presses (before the modifiers are worked out from them)
        // and wheel commands; the rest of the input goes on to the app. Its touch router owns
        // every finger: gestures move the view and taps run the Touch Input shortcuts, a lone
        // finger becomes the pointer after a moment, and palms wait while the pen is near.
        let config = self.touch.config();
        // The pen erases while a control set to erase is in use: the Eraser tool is selected
        // for it (before this frame's pen events are consumed) and the previous tool comes
        // back after. The flag comes from the pen hook's samples.
        let erasing = self.pen_erasing.load(std::sync::atomic::Ordering::Relaxed) && self.app.session.active_index().is_some();
        if erasing && self.erasing_tool.is_none() {
            let current = self.app.session.tool_id();
            self.erasing_tool = (current != "eraser").then_some(current);
            if current != "eraser"
                && let Err(e) = self.app.run("tool.select", serde_json::json!({ "tool": "eraser" }))
            {
                log::warn!("pen eraser: {e}");
            }
        } else if !erasing
            && let Some(tool) = self.erasing_tool.take()
            && let Err(e) = self.app.run("tool.select", serde_json::json!({ "tool": tool }))
        {
            log::warn!("pen eraser: {e}");
        }
        // The pen's side buttons and eraser end (Touch Input settings): erasing is the shell's
        // (above), right-clicks while touching are the router's, presses are handled here.
        vectorcraft_tablet::android::set_erasing_controls(config.erasing_controls());
        let presses = vectorcraft_tablet::android::take_presses();
        for p in &presses {
            let setting = config.pen(p.control);
            if setting.action != PenAction::Shortcut {
                continue;
            }
            match crate::pad::config::parse(&setting.shortcut) {
                Ok(chord) if !chord.is_empty() => {
                    raw.events.extend(crate::pad::press(chord));
                    raw.events.extend(crate::pad::release(chord));
                }
                Ok(_) => {}
                Err(e) => log::warn!("pen {:?}: {e}", p.control),
            }
        }
        // A pen or mouse presses the secondary button: a mouse's right button, a pen's side
        // button set to right-click (hovering shows a context menu; while touching, the
        // router's hold and drag work with it).
        let secondary = |id: egui::TouchId| {
            let buttons = vectorcraft_tablet::android::pointer_buttons(id.0);
            match vectorcraft_tablet::android::pointer_tool(id.0) {
                Some(t) if t == vectorcraft_tablet::motion::tool::MOUSE => buttons & vectorcraft_tablet::motion::button::SECONDARY != 0,
                Some(t) => {
                    let used = vectorcraft_tablet::motion::controls(t, buttons);
                    PenControl::ALL.iter().any(|c| used[c.index()] && config.pen(*c).action == PenAction::RightClick)
                }
                None => false,
            }
        };
        let env = crate::pad::router::Env {
            // Gestures move the canvas only while a document is open.
            canvas: self.app.session.active_index().map(|_| self.app.canvas_rect).flatten(),
            is_finger: &|id: egui::TouchId| vectorcraft_tablet::android::is_finger(id.0),
            pen_near: vectorcraft_tablet::android::pen_seen_within(std::time::Duration::from_millis(300)),
            config: &config,
            list_item: &|p: egui::Pos2| self.drawn.list_item(p),
            secondary: &secondary,
            // No sideways-scrolling strip in the layout yet (the Control panel doesn't
            // overflow): the router's strip handling stays switched off.
            scroll_strip: None,
            now: raw.time.unwrap_or_else(|| ctx.input(|i| i.time)),
        };
        // Every pen sample's position, not just each event's latest (see `pen_path`). Samples
        // wait for a frame with the pen's touch in it; a new touch-down drops older ones.
        let is_pen = |id: egui::TouchId| {
            use vectorcraft_tablet::motion::tool;
            matches!(vectorcraft_tablet::android::pointer_tool(id.0), Some(tool::STYLUS | tool::ERASER))
        };
        if raw.events.iter().any(|e| matches!(e, egui::Event::Touch { id, phase: egui::TouchPhase::Start, .. } if is_pen(*id))) {
            self.pen_path.clear();
        }
        self.pen_path.extend(vectorcraft_tablet::android::take_pen_path());
        if self.pen_path.len() > 480 {
            let extra = self.pen_path.len() - 480;
            self.pen_path.drain(..extra);
        }
        if crate::pen_path::splice(&mut raw.events, &self.pen_path, ctx.pixels_per_point(), is_pen) > 0 {
            self.pen_path.clear();
        }
        let drawn = &self.drawn;
        self.pad.input(ctx, raw, &env, |p| drawn.covered(p));
        if self.pad.take_settings_tap() {
            self.settings_open = true;
        }
        // A side button pressed while the pen hovers: a right-click under the pen (after the
        // router, which replaces pointer events with its own).
        let ppp = ctx.pixels_per_point();
        for p in presses {
            if let (Some((x, y)), PenAction::RightClick, PenControl::Lower | PenControl::Upper | PenControl::Top) =
                (p.hover, config.pen(p.control).action, p.control)
            {
                let pos = egui::pos2(x / ppp, y / ppp);
                raw.events.push(egui::Event::PointerMoved(pos));
                for pressed in [true, false] {
                    raw.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
            }
        }
        self.keys.apply(raw);
        self.app.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let tokens = vectorcraft_ui_egui::theme::Tokens::get(ui.ctx());
        // The pad first, so the app lays out beside it.
        self.pad.show(ui, &tokens, &self.app);
        self.app.ui(ui);
        // Preferences are written when confirmed, not only at exit: a session ended any other
        // way (the system shutting down, the process killed or crashing) used to lose them (#1747).
        if self.app.take_prefs_changed() {
            self.save_prefs();
        }
        // The settings window (the pad's gear key): the pad's settings, with the Touch Input
        // section under them.
        if self.settings_open {
            let mut open = true;
            let center = ui.ctx().content_rect().center();
            egui::Window::new("Shortcut Pad")
                .open(&mut open)
                .collapsible(false)
                .resizable([true, true])
                .default_width(380.0)
                .pivot(egui::Align2::CENTER_CENTER)
                .default_pos(center)
                .show(ui.ctx(), |ui| {
                    self.pad.settings_body(ui, &tokens);
                    self.touch.settings_body(
                        ui,
                        &tokens,
                        vectorcraft_tablet::android::controls_held(),
                        vectorcraft_tablet::android::last_buttons(),
                    );
                });
            self.settings_open = open;
        }
        // What was drawn where, for the next frame's touches (see `touch::Snapshot`).
        self.drawn = crate::touch::Snapshot::take(ui.ctx());
    }

    #[cfg_attr(target_os = "android", allow(unused_mut))]
    fn on_exit(&mut self) {
        // The surface ended by itself (the Activity went away): keep what the last frames had.
        self.save_prefs();
    }
}
