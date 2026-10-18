//! Touch Input: multi-touch gestures on the canvas and fingers in the interface. Ported from
//! the photocraft-android shell (the same touch router), adapted to VectorCraft's views.
//!
//! - **On the canvas:** two fingers pan, pinch to zoom and twist to rotate the view, all at once
//!   (the art stays under the fingers); a quick two-finger tap undoes and a three-finger tap
//!   redoes (any shortcut can be chosen). One finger draws like the pen, moves the canvas, or
//!   does nothing.
//! - **Palm rejection:** fingers are ignored while the pen is down or near the screen.
//! - **Menus, panels and lists:** a flick keeps scrolling, and two fingers scroll.
//! - The settings live in the Shortcut Pad settings window (the pad's gear key).

pub mod config;

use crate::pad::config::{self as pad_config, PRESETS};
use crate::pad::router::ViewMove;
use config::TouchConfig;
use vectorcraft_ui_egui::VectorcraftApp;
use vectorcraft_ui_egui::canvas::Xf;
use vectorcraft_ui_egui::theme::Tokens;

/// The zoom range of the canvas (as the Shortcut Pad's wheel allows), like the app's own zoom
/// stops (3.13%–640%).
const ZOOM_RANGE: std::ops::RangeInclusive<f64> = 0.0313..=640.0;

pub struct TouchInput {
    config: TouchConfig,
    path: std::path::PathBuf,
}

impl TouchInput {
    pub fn new(files_dir: &std::path::Path) -> Self {
        let path = files_dir.join("touch-input.json");
        let config = TouchConfig::load(&path);
        Self { config, path }
    }

    pub fn config(&self) -> TouchConfig {
        self.config.clone()
    }

    /// Saves the given config, the way the Shortcut Pad's window does after each change.
    pub fn set_config(&mut self, config: TouchConfig) {
        self.config = config.sanitized();
        if let Err(e) = self.config.save(&self.path) {
            log::warn!("couldn't save the touch input settings: {e}");
        }
    }

    /// The Touch Input section of the Shortcut Pad settings window.
    pub fn settings_body(&mut self, ui: &mut egui::Ui, tk: &Tokens) {
        let before = self.config.clone();
        let mut c = before.clone();
        ui.separator();
        ui.strong("Touch Input");
        ui.strong("One finger on the canvas");
        for m in config::FingerMode::ALL {
            ui.radio_value(&mut c.finger, m, m.label());
        }
        ui.separator();
        ui.strong("Two fingers on the canvas");
        ui.checkbox(&mut c.pan, "Drag to move the canvas");
        ui.checkbox(&mut c.zoom, "Pinch to zoom");
        ui.checkbox(&mut c.rotate, "Twist to rotate the view");
        ui.separator();
        ui.strong("Taps on the canvas");
        egui::Grid::new("touch_taps").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
            tap_row(ui, tk, "Two-finger tap", "touch_two_tap", &mut c.two_finger_tap);
            tap_row(ui, tk, "Three-finger tap", "touch_three_tap", &mut c.three_finger_tap);
        });
        ui.separator();
        ui.checkbox(&mut c.palm_rejection, "Palm rejection: ignore fingers while the pen is on or near the screen");
        ui.checkbox(&mut c.two_finger_scroll, "Two fingers scroll menus, panels and lists");
        ui.separator();
        if ui.button("Restore Touch Defaults").clicked() {
            c = TouchConfig::default();
        }
        if c != before {
            self.set_config(c);
        }
    }
}

/// A tap's row: its name, a preset menu and the shortcut (typed or chosen).
fn tap_row(ui: &mut egui::Ui, tk: &Tokens, name: &str, salt: &str, shortcut: &mut String) {
    ui.label(name);
    let current =
        PRESETS.iter().find(|(_, _, s)| s.eq_ignore_ascii_case(shortcut)).map_or(if shortcut.is_empty() { "Nothing" } else { "Custom" }, |(n, _, _)| *n);
    egui::ComboBox::from_id_salt(salt).selected_text(current).width(150.0).show_ui(ui, |ui| {
        if ui.selectable_label(shortcut.is_empty(), "Nothing").clicked() {
            shortcut.clear();
        }
        for (menu_name, _, s) in PRESETS {
            if ui.selectable_label(shortcut.eq_ignore_ascii_case(s), format!("{menu_name}  ({s})")).clicked() {
                *shortcut = (*s).into();
            }
        }
    });
    ui.vertical(|ui| {
        ui.add_sized([130.0, 24.0], egui::TextEdit::singleline(shortcut).hint_text("e.g. Ctrl+Z"));
        if let Err(e) = pad_config::parse(shortcut) {
            ui.colored_label(tk.error, e);
        }
    });
    ui.end_row();
}

/// The tallest widget treated as a list row (taller ones, such as a gradient ramp, keep their
/// own vertical drags).
const LIST_ROW_MAX_HEIGHT: f32 = 120.0;

/// One widget as laid out in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Widget {
    pub rect: egui::Rect,
    pub interact: egui::Rect,
    pub click: bool,
    pub drag: bool,
    pub enabled: bool,
}

/// Where everything was drawn last frame, layer by layer (back to front), for the touch router
/// to decide what a new touch landed on. Taken at the end of the frame's UI ([`Snapshot::take`]):
/// between frames, where the router runs, egui's own lookups (`layer_id_at`, the widget rects)
/// can miss panels and popups such as the color picker, which would let a touch on it count as
/// one on the canvas behind it.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    layers: Vec<(egui::Order, Vec<Widget>)>,
}

impl Snapshot {
    pub fn take(ctx: &egui::Context) -> Self {
        let order: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
        ctx.viewport(|vp| {
            let w = &vp.this_pass.widgets;
            let widgets = |l: egui::LayerId| -> Vec<Widget> {
                w.get_layer(l)
                    .map(|w| Widget { rect: w.rect, interact: w.interact_rect, click: w.sense.senses_click(), drag: w.sense.senses_drag(), enabled: w.enabled })
                    .collect()
            };
            let mut layers: Vec<(egui::Order, Vec<Widget>)> = order.iter().map(|l| (l.order, widgets(*l))).collect();
            // Layers new this frame aren't in the area order yet: on top, by their order.
            let mut extra: Vec<egui::LayerId> = w.layer_ids().filter(|l| !order.contains(l)).collect();
            extra.sort_by_key(|l| l.order);
            layers.extend(extra.into_iter().map(|l| (l.order, widgets(l))));
            layers.retain(|(_, w)| !w.is_empty());
            Self { layers }
        })
    }

    #[cfg(test)]
    pub fn from_layers(layers: Vec<(egui::Order, Vec<Widget>)>) -> Self {
        Self { layers }
    }

    /// The topmost layer with something drawn at `p`.
    fn top(&self, p: egui::Pos2) -> Option<&(egui::Order, Vec<Widget>)> {
        self.layers.iter().rev().find(|(_, w)| w.iter().any(|w| w.rect.contains(p)))
    }

    /// Whether a window, menu or popup was drawn over `p`: touches there belong to it, not to
    /// the canvas or the Shortcut Pad.
    pub fn covered(&self, p: egui::Pos2) -> bool {
        self.top(p).is_some_and(|(order, _)| *order != egui::Order::Background)
    }

    /// Whether `p` is on a list row inside a scrolling list: a widget that can be both clicked
    /// and dragged (a Layers panel row that reorders, a preset card, a slider) on top of a
    /// larger drag-only area containing it (a scroll area's content). A finger swiping up or
    /// down there scrolls the list instead of dragging the row.
    pub fn list_item(&self, p: egui::Pos2) -> bool {
        let Some((_, widgets)) = self.top(p) else { return false };
        let hits: Vec<&Widget> = widgets.iter().filter(|w| w.enabled && w.interact.contains(p)).collect();
        let Some(row) = hits.iter().rev().find(|w| w.click && w.drag) else { return false };
        if row.rect.height() > LIST_ROW_MAX_HEIGHT {
            return false;
        }
        // A scroll area's drag-only content area under the row, or (a scroll area that registers
        // none, like a panel's) content overflowing a box that shows part of it.
        let all: Vec<&Widget> = widgets.iter().filter(|w| w.rect.contains(p)).collect();
        let dragged_area = hits.iter().any(|w| w.drag && !w.click && w.rect.height() > row.rect.height() * 1.5);
        // Scrolled content overlaps the box showing it and runs past it: neither holds the other
        // (a parent box holds its children; it does not scroll them).
        let overflowing = all.iter().any(|content| {
            content.rect.height() > row.rect.height() * 1.5
                && all.iter().any(|view| {
                    view.rect.height() > row.rect.height()
                        && view.rect.height() < content.rect.height()
                        && !view.rect.contains_rect(content.rect)
                        && !content.rect.contains_rect(view.rect)
                })
        });
        dragged_area || overflowing
    }
}

/// The view after a gesture: `m` applied to `xf` (the active canvas mapping). Returns the new
/// zoom factor relative to now (`> 1`: closer), rotation in degrees and centre, keeping the
/// document point under `m.from` under `m.to`. VectorCraft's `Xf` has no mirrored view, so a
/// clockwise twist always turns the view clockwise.
pub fn moved_view(xf: &Xf, zoom: f64, m: &ViewMove) -> (f64, f64, vectorcraft_geom::Point) {
    let doc = xf.to_doc(m.from);
    let zoom = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
    let scale = if m.scale.is_finite() && m.scale > 0.0 { m.scale as f64 } else { 1.0 };
    let new_zoom = (zoom * scale).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
    let turn = if m.rotate.is_finite() { m.rotate as f64 } else { 0.0 };
    // VectorCraft keeps the view rotation normalized (`vectorcraft_geom::normalize_deg`).
    let rotation = vectorcraft_geom::normalize_deg(xf.rot.to_degrees() + turn);
    // The zoom and turn about the old centre alone move the anchor (`m.to`'s document point
    // without the pan): where they leave it decides where the centre must go so that the art
    // under the fingers stays under the fingers. `to_doc` adds the centre back in, so it is
    // subtracted again here (the plain rotated/scaled part is what anchors).
    let turned = Xf { rect: xf.rect, zoom: new_zoom, center: xf.center, rot: rotation.to_radians() };
    let off = turned.to_doc(m.to);
    let center = vectorcraft_geom::Point::new(doc.x - (off.x - xf.center.x), doc.y - (off.y - xf.center.y));
    (new_zoom, rotation, center)
}

/// Apply gesture moves to the active document's view: the art stays under the fingers
/// (`moved_view` decides where the centre goes), and the view the document opens with is
/// superseded by the user's.
pub fn apply(app: &mut VectorcraftApp, moves: &[ViewMove]) {
    if moves.is_empty() {
        return;
    }
    for m in moves {
        let Some(rect) = app.canvas_rect else { return };
        let Some(v) = app.view_mut() else { return };
        let (zoom, rotation, center) = moved_view(&Xf::new(rect, v), v.zoom, m);
        if !(zoom.is_finite() && rotation.is_finite() && center.x.is_finite() && center.y.is_finite()) {
            return;
        }
        let Some(v) = app.view_mut() else { return };
        v.fitted = true;
        v.zoom = zoom;
        v.rotation = rotation;
        v.center = center;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pad::router::ViewMove;
    use egui::{pos2, Rect};
    use vectorcraft_geom::Point;

    /// A canvas mapping: the canvas somewhere mid-window, at `zoom` with the view centred on the
    /// document point `center`, turned `rot_deg`.
    fn xf(zoom: f64, center: (f64, f64), rot_deg: f64) -> Xf {
        Xf {
            rect: Rect::from_min_size(pos2(100.0, 50.0), egui::vec2(900.0, 700.0)),
            zoom,
            center: Point::new(center.0, center.1),
            rot: rot_deg.to_radians(),
        }
    }

    /// The gestures' promise: the art under the fingers stays there. `after` is the mapping
    /// `moved_view` built for `before` and a move from `from` to `to`: the document point that
    /// was under `from` ends up under `to` (the same place when the fingers hold still).
    fn anchored(before: &Xf, after_zoom: f64, after_rot: f64, after_center: Point, from: egui::Pos2, to: egui::Pos2) {
        let doc = before.to_doc(from);
        let after = Xf { rect: before.rect, zoom: after_zoom, center: after_center, rot: after_rot.to_radians() };
        let back = after.to_screen(doc);
        let (dx, dy) = ((back.x - to.x) as f64, (back.y - to.y) as f64);
        assert!((dx * dx + dy * dy).sqrt() < 0.5, "doc point {:?} went to {back:?} instead of {to:?}", doc);
    }

    #[test]
    fn a_starting_gesture_does_not_move_the_view() {
        // The first move of a gesture: the fingers moved 10 points, the spread and angle barely
        // changed (that is when they pass the lock thresholds). Before the fix this centred the
        // view on the document's origin — the artboard's top-left corner snapped to mid-screen.
        let before = xf(2.0, (306.0, 396.0), 0.0);
        let (zoom, rot, center) = moved_view(&before, 2.0, &ViewMove { from: pos2(500.0, 400.0), to: pos2(510.0, 405.0), scale: 1.02, rotate: 3.0 });
        let moved = ((center.x - 306.0).abs() + (center.y - 396.0).abs()) < 30.0;
        assert!(moved, "a 10-point, 3° nudge jumped the centre to {:?} — the view snapped", center);
        anchored(&before, zoom, rot, center, pos2(500.0, 400.0), pos2(510.0, 405.0));
    }

    #[test]
    fn panning_moves_the_canvas_with_the_fingers() {
        let before = xf(2.0, (306.0, 396.0), 0.0);
        let (zoom, rot, center) = moved_view(&before, 2.0, &ViewMove { from: pos2(500.0, 400.0), to: pos2(600.0, 400.0), scale: 1.0, rotate: 0.0 });
        assert_eq!((zoom, rot), (2.0, 0.0));
        assert!(((center.x - 256.0) < 1e-6) && ((center.y - 396.0) < 1e-6), "fingers 100 px right at zoom 2: centre 50 pt left, was {center:?}");
        anchored(&before, zoom, rot, center, pos2(500.0, 400.0), pos2(600.0, 400.0));
    }

    #[test]
    fn pinching_zooms_about_the_fingers() {
        // Spread from 200 to 420 points around (460, 400): ×2.1, and the art under the fingers
        // stays there (the zoom's anchor is the fingers).
        let before = xf(2.0, (306.0, 396.0), 0.0);
        let (zoom, rot, center) = moved_view(&before, 2.0, &ViewMove { from: pos2(560.0, 400.0), to: pos2(560.0, 400.0), scale: 2.1, rotate: 0.0 });
        assert!(((zoom - 4.2) / 4.2) < 1e-6, "zoom ×2.1, was {zoom}");
        assert_eq!(rot, 0.0);
        assert!(center.x > 306.0, "zooming in on a point right of centre pushes the centre right");
        anchored(&before, zoom, rot, center, pos2(560.0, 400.0), pos2(560.0, 400.0));
    }

    #[test]
    fn twisting_rotates_the_view_about_the_fingers() {
        let before = xf(2.0, (306.0, 396.0), 0.0);
        let (zoom, rot, center) = moved_view(&before, 2.0, &ViewMove { from: pos2(500.0, 400.0), to: pos2(500.0, 400.0), scale: 1.0, rotate: 45.0 });
        assert_eq!(zoom, 2.0);
        assert!((rot - 45.0).abs() < 1e-6, "45°, was {rot}");
        anchored(&before, zoom, rot, center, pos2(500.0, 400.0), pos2(500.0, 400.0));
    }

    /// All at once, on a view already turned: the sign must show on screen (twisting clockwise
    /// on the screen turns the art clockwise as seen, whatever the view started at).
    #[test]
    fn a_rotated_view_still_anchors_and_twists_on_screen() {
        let before = xf(1.0, (306.0, 396.0), 30.0);
        let (zoom, rot, center) = moved_view(&before, 1.0, &ViewMove { from: pos2(450.0, 350.0), to: pos2(520.0, 430.0), scale: 1.3, rotate: 17.0 });
        assert!(((zoom - 1.3) / 1.3) < 1e-6 && ((rot - 47.0).abs() < 1e-6), "was ({zoom}, {rot})");
        anchored(&before, zoom, rot, center, pos2(450.0, 350.0), pos2(520.0, 430.0));
    }

    #[test]
    fn hostile_moves_do_not_move_the_view() {
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            for rotate in [f32::NAN, f32::INFINITY] {
                let before = xf(2.0, (306.0, 396.0), 0.0);
                let (zoom, rot, center) = moved_view(&before, 2.0, &ViewMove { from: pos2(500.0, 400.0), to: pos2(600.0, 400.0), scale, rotate });
                // The broken scale and rotation fall back to none; the plain pan still happens.
                assert_eq!((zoom, rot, center), (2.0, 0.0, Point::new(256.0, 396.0)), "scale {scale}, rotate {rotate}");
                anchored(&before, zoom, rot, center, pos2(500.0, 400.0), pos2(600.0, 400.0));
            }
        }
    }

    #[test]
    fn the_zoom_range_clamps() {
        let before = xf(1.0, (306.0, 396.0), 0.0);
        let (zoom, _, _) = moved_view(&before, 1000.0, &ViewMove { from: pos2(500.0, 400.0), to: pos2(500.0, 400.0), scale: 1e6, rotate: 0.0 });
        assert_eq!(zoom, *ZOOM_RANGE.end());
        let (zoom, _, _) = moved_view(&before, 0.04, &ViewMove { from: pos2(500.0, 400.0), to: pos2(500.0, 400.0), scale: 1e-9, rotate: 0.0 });
        assert_eq!(zoom, *ZOOM_RANGE.start());
    }
}
