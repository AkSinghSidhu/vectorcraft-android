//! Touch Input settings: what fingers do on the canvas, the multi-touch gestures and what the
//! taps run. Saved as JSON in the app's files directory, like the Shortcut Pad's.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// What one finger does on the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FingerMode {
    /// Paints and clicks like the mouse (the pen does too).
    #[default]
    Draw,
    /// Drags the canvas around; only the pen paints.
    Navigate,
    /// Nothing: only the pen paints, and fingers are for gestures.
    Gestures,
}

impl FingerMode {
    pub const ALL: [FingerMode; 3] = [FingerMode::Draw, FingerMode::Navigate, FingerMode::Gestures];

    /// Long form for the Touch Input settings section.
    pub fn label(self) -> &'static str {
        match self {
            FingerMode::Draw => "Draw and click, like the pen",
            FingerMode::Navigate => "Move the canvas",
            FingerMode::Gestures => "Nothing (gestures only)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TouchConfig {
    /// One finger on the canvas.
    pub finger: FingerMode,
    /// Two fingers dragging move the canvas.
    pub pan: bool,
    /// Pinching zooms around the fingers.
    pub zoom: bool,
    /// Twisting two fingers rotates the view.
    pub rotate: bool,
    /// The shortcut a two-finger tap presses (empty: nothing).
    pub two_finger_tap: String,
    /// The shortcut a three-finger tap presses (empty: nothing).
    pub three_finger_tap: String,
    /// Ignore fingers while the pen is touching or near the screen.
    pub palm_rejection: bool,
    /// Two fingers dragging over menus, panels and lists scroll them.
    pub two_finger_scroll: bool,
}

impl Default for TouchConfig {
    fn default() -> Self {
        Self {
            finger: FingerMode::Draw,
            pan: true,
            zoom: true,
            rotate: true,
            two_finger_tap: "Ctrl+Z".into(),
            three_finger_tap: "Ctrl+Shift+Z".into(),
            palm_rejection: true,
            two_finger_scroll: true,
        }
    }
}

impl TouchConfig {
    pub fn sanitized(mut self) -> Self {
        for s in [&mut self.two_finger_tap, &mut self.three_finger_tap] {
            *s = s.trim().chars().take(40).collect();
        }
        self
    }

    pub fn load(path: &Path) -> Self {
        let read = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<TouchConfig>(&b).map_err(|e| log::warn!("touch input settings: {e}")).ok());
        read.unwrap_or_default().sanitized()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        vectorcraft_engine::cmd::fileio::write_atomic(path, &text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_broken_settings_fall_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("vectorcraft-touch-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("touch-input.json");
        assert_eq!(TouchConfig::load(&path), TouchConfig::default());
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(TouchConfig::load(&path), TouchConfig::default());
        // Older files without a field keep the default for it.
        std::fs::write(&path, br#"{"finger":"Navigate"}"#).unwrap();
        let c = TouchConfig::load(&path);
        assert_eq!(c.finger, FingerMode::Navigate);
        assert!(c.zoom && c.palm_rejection);
        let saved = TouchConfig { finger: FingerMode::Gestures, two_finger_tap: "  Ctrl+0 ".into(), ..Default::default() }.sanitized();
        saved.save(&path).unwrap();
        assert_eq!(TouchConfig::load(&path).two_finger_tap, "Ctrl+0");
        std::fs::remove_dir_all(dir).ok();
    }
}
