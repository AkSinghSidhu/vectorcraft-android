//! The pad's wheel: a finger sliding around the ring turns, and the current function turns the
//! movement into changes. Clockwise zooms in, rotates the view clockwise, grows the freehand
//! tool's size, or redoes; counter-clockwise the opposite.
//!
//! Zoom, rotation and freehand size follow the finger continuously (no jumps between preset
//! zoom levels or sizes); history moves one state per [`history_detent`]. The user's
//! sensitivity scales every function.

use super::config::WheelMode;

/// Degrees between the ring's tick marks.
pub const TICK: f32 = 15.0;

/// At sensitivity 1: a quarter turn doubles the zoom, a third of a turn doubles the freehand
/// size, and the view rotates as far as the finger.
const ZOOM_DOUBLING_DEG: f32 = 90.0;
const SIZE_DOUBLING_DEG: f32 = 120.0;
const HISTORY_DEG: f32 = 30.0;

/// Degrees of ring travel per history step at `sensitivity`.
pub fn history_detent(sensitivity: f32) -> f32 {
    (HISTORY_DEG / sensitivity.max(0.01)).clamp(8.0, 120.0)
}

/// What a movement of the ring asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Multiply the view's zoom.
    Zoom(f32),
    /// Rotate the view by degrees.
    Rotate(f32),
    /// Multiply the freehand size (0.1–1000 pt, like the tool's option).
    Size(f32),
    /// Run a command (undo, redo).
    Command(&'static str),
}

/// The angle of `p` around `center` in degrees, clockwise on screen (y points down) from the
/// positive x axis.
pub fn angle(center: egui::Pos2, p: egui::Pos2) -> f32 {
    (p.y - center.y).atan2(p.x - center.x).to_degrees()
}

/// One finger's turn of the ring.
#[derive(Clone, Copy, Debug, Default)]
pub struct Turn {
    last: Option<f32>,
    /// History travel not yet spent on a step.
    pending: f32,
}

impl Turn {
    /// The finger is at `angle` (`None`: too near the centre to read an angle; the next real
    /// angle starts afresh instead of jumping): the actions for the movement since the last call.
    pub fn moved(&mut self, mode: WheelMode, angle: Option<f32>, sensitivity: f32) -> Vec<Action> {
        let Some(angle) = angle.filter(|a| a.is_finite()) else {
            self.last = None;
            return Vec::new();
        };
        let Some(last) = self.last.replace(angle) else { return Vec::new() };
        // The short way round: crossing ±180° is a small step, not a full turn back.
        let delta = (angle - last + 540.0).rem_euclid(360.0) - 180.0;
        if delta.abs() < 0.01 {
            return Vec::new();
        }
        let s = if sensitivity.is_finite() { sensitivity.clamp(0.05, 20.0) } else { 1.0 };
        let turned = delta * s;
        match mode {
            WheelMode::Zoom => vec![Action::Zoom(2f32.powf(turned / ZOOM_DOUBLING_DEG))],
            WheelMode::Rotate => vec![Action::Rotate(turned)],
            WheelMode::Freehand => vec![Action::Size(2f32.powf(turned / SIZE_DOUBLING_DEG))],
            WheelMode::History => {
                let detent = history_detent(s);
                self.pending += delta;
                let steps = (self.pending / detent).trunc();
                self.pending -= steps * detent;
                let id = if steps > 0.0 { "edit.redo" } else { "edit.undo" };
                (0..steps.abs().min(24.0) as usize).map(|_| Action::Command(id)).collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_functions_follow_the_finger() {
        let mut t = Turn::default();
        assert!(t.moved(WheelMode::Zoom, Some(0.0), 1.0).is_empty(), "the first touch only sets the start");
        let v = t.moved(WheelMode::Zoom, Some(90.0), 1.0);
        assert!(matches!(v.as_slice(), [Action::Zoom(f)] if (f - 2.0).abs() < 1e-4), "a quarter turn doubles: {v:?}");
        let v = t.moved(WheelMode::Zoom, Some(45.0), 2.0);
        assert!(matches!(v.as_slice(), [Action::Zoom(f)] if (f - 0.5).abs() < 1e-4), "sensitivity 2: back 45° halves: {v:?}");
        // Crossing the seam (175° → 185°) is +10°, not −350°.
        let mut t = Turn::default();
        t.moved(WheelMode::Rotate, Some(175.0), 1.0);
        assert_eq!(t.moved(WheelMode::Rotate, Some(-175.0), 1.0), vec![Action::Rotate(10.0)]);
        // Near the centre: no reading, and no jump when the finger comes back out.
        assert!(t.moved(WheelMode::Rotate, None, 1.0).is_empty());
        assert!(t.moved(WheelMode::Rotate, Some(0.0), 1.0).is_empty(), "starts afresh");
    }

    #[test]
    fn history_steps_per_detent() {
        let mut t = Turn::default();
        t.moved(WheelMode::History, Some(0.0), 1.0);
        assert_eq!(t.moved(WheelMode::History, Some(-31.0), 1.0), vec![Action::Command("edit.undo")]);
        let mut t = Turn::default();
        t.moved(WheelMode::History, Some(0.0), 2.0);
        assert_eq!(t.moved(WheelMode::History, Some(31.0), 2.0).len(), 2, "sensitivity 2: 15° per step");
    }

    #[test]
    fn the_freehand_size_grows_and_shrinks() {
        let mut t = Turn::default();
        t.moved(WheelMode::Freehand, Some(0.0), 1.0);
        let v = t.moved(WheelMode::Freehand, Some(120.0), 1.0);
        assert!(matches!(v.as_slice(), [Action::Size(f)] if (f - 2.0).abs() < 1e-4), "a third of a turn doubles: {v:?}");
    }

    #[test]
    fn hostile_input_does_nothing_odd() {
        let mut t = Turn::default();
        t.moved(WheelMode::Zoom, Some(0.0), 1.0);
        assert!(t.moved(WheelMode::Zoom, Some(f32::NAN), 1.0).is_empty(), "a NaN angle reads as no angle");
        t.moved(WheelMode::Zoom, Some(0.0), 1.0);
        let v = t.moved(WheelMode::Zoom, Some(30.0), f32::NAN);
        assert!(matches!(v.as_slice(), [Action::Zoom(f)] if f.is_finite()));
    }
}
