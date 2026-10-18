//! The Shortcut Pad's settings: which side it sits on, what its keys (2 to 10) press and which
//! functions the wheel's centre button cycles through. Saved as JSON in the app's files
//! directory; a missing or unreadable file gives the defaults. Ported from the photocraft
//! shell; the wheel functions are VectorCraft's (no raster brush here).

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The fewest and most keys the pad can have (split around the wheel, the extra one of an odd
/// count above it). Ten still fit a tablet's height at a comfortable size (see
/// `router::Layout`).
pub const MIN_KEYS: usize = 2;
pub const MAX_KEYS: usize = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    #[default]
    Right,
}

/// One key: the text it shows and the shortcut it presses (`"Ctrl+Z"`, `"V"`, or a modifier on
/// its own, `"Alt"`, held for as long as the key is touched). An empty shortcut does nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeySlot {
    pub label: String,
    pub shortcut: String,
}

impl KeySlot {
    fn new(label: &str, shortcut: &str) -> Self {
        Self { label: label.into(), shortcut: shortcut.into() }
    }
}

/// What turning the wheel does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WheelMode {
    Zoom,
    Rotate,
    /// The size of the freehand tools (Pencil, Paintbrush, Blob Brush, Eraser).
    Freehand,
    History,
}

impl WheelMode {
    pub const ALL: [WheelMode; 4] = [WheelMode::Zoom, WheelMode::Rotate, WheelMode::Freehand, WheelMode::History];

    /// The name in the settings.
    pub fn label(self) -> &'static str {
        match self {
            WheelMode::Zoom => "Zoom",
            WheelMode::Rotate => "Rotate View",
            WheelMode::Freehand => "Freehand Size",
            WheelMode::History => "Undo / Redo",
        }
    }

    /// The short name on the wheel's centre button.
    pub fn short(self) -> &'static str {
        match self {
            WheelMode::Zoom => "Zoom",
            WheelMode::Rotate => "Rotate",
            WheelMode::Freehand => "Size",
            WheelMode::History => "Undo",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PadConfig {
    pub enabled: bool,
    pub side: Side,
    pub keys: Vec<KeySlot>,
    /// The functions the centre button cycles through, in order (never empty once
    /// [`Self::sanitized`]).
    pub wheel_modes: Vec<WheelMode>,
    /// Pressing the wheel opens a large ring to turn (easier to hit and smoother to turn than
    /// the small one on the pad).
    pub enlarge: bool,
    /// The large ring's diameter in points ([`WHEEL_SIZE_RANGE`]).
    pub wheel_size: f32,
    /// How much a turn changes (1: a quarter turn doubles the zoom; [`SENSITIVITY_RANGE`]).
    pub sensitivity: f32,
}

/// The large ring's diameter range in points.
pub const WHEEL_SIZE_RANGE: std::ops::RangeInclusive<f32> = 140.0..=400.0;
/// The wheel sensitivity range.
pub const SENSITIVITY_RANGE: std::ops::RangeInclusive<f32> = 0.25..=4.0;

impl Default for PadConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            side: Side::Right,
            keys: vec![KeySlot::new("Undo", "Ctrl+Z"), KeySlot::new("Redo", "Ctrl+Shift+Z"), KeySlot::new("Alt", "Alt"), KeySlot::new("Pan", "Space")],
            wheel_modes: vec![WheelMode::Zoom, WheelMode::Rotate, WheelMode::Freehand],
            enlarge: true,
            wheel_size: 150.0,
            sensitivity: 1.0,
        }
    }
}

impl PadConfig {
    /// Between [`MIN_KEYS`] and [`MAX_KEYS`] keys, at least one wheel function, no function
    /// twice.
    pub fn sanitized(mut self) -> Self {
        let defaults = Self::default();
        let n = self.keys.len().clamp(MIN_KEYS, MAX_KEYS);
        self.set_key_count(n);
        let mut seen = Vec::new();
        self.wheel_modes.retain(|m| {
            let new = !seen.contains(m);
            seen.push(*m);
            new
        });
        if self.wheel_modes.is_empty() {
            self.wheel_modes = defaults.wheel_modes;
        }
        let clamp = |v: f32, r: &std::ops::RangeInclusive<f32>, d: f32| if v.is_finite() { v.clamp(*r.start(), *r.end()) } else { d };
        self.wheel_size = clamp(self.wheel_size, &WHEEL_SIZE_RANGE, defaults.wheel_size);
        self.sensitivity = clamp(self.sensitivity, &SENSITIVITY_RANGE, defaults.sensitivity);
        self
    }

    /// Grow or shrink to `n` keys (clamped to the allowed range). Removed keys go from the
    /// bottom; added ones take the presets no key uses yet, in order.
    pub fn set_key_count(&mut self, n: usize) {
        let n = n.clamp(MIN_KEYS, MAX_KEYS);
        self.keys.truncate(n);
        while self.keys.len() < n {
            let next = PRESETS
                .iter()
                .find(|(_, _, shortcut)| !self.keys.iter().any(|k| k.shortcut.eq_ignore_ascii_case(shortcut)))
                .map_or_else(|| KeySlot::new("", ""), |(_, label, shortcut)| KeySlot::new(label, shortcut));
            self.keys.push(next);
        }
    }

    pub fn load(path: &Path) -> Self {
        let read = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<PadConfig>(&b).map_err(|e| log::warn!("shortcut pad settings: {e}")).ok());
        read.unwrap_or_default().sanitized()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        vectorcraft_engine::cmd::fileio::write_atomic(path, &text).map_err(|e| e.to_string())
    }
}

/// Shortcuts the settings offer: (name in the menu, label on the key, shortcut). Key labels
/// stay within 8 characters so they fit the key; any other shortcut can be typed. VectorCraft
/// tool keys (Photoshop's do not apply here): see `vectorcraft_tools::catalog`.
pub const PRESETS: &[(&str, &str, &str)] = &[
    ("Undo", "Undo", "Ctrl+Z"),
    ("Redo", "Redo", "Ctrl+Shift+Z"),
    ("Alt (hold)", "Alt", "Alt"),
    ("Shift (hold)", "Shift", "Shift"),
    ("Ctrl (hold)", "Ctrl", "Ctrl"),
    ("Pan (hold Space)", "Pan", "Space"),
    ("Selection", "Select", "V"),
    ("Direct Select", "Direct", "A"),
    ("Pen", "Pen", "P"),
    ("Pencil", "Pencil", "N"),
    ("Paintbrush", "Brush", "B"),
    ("Blob Brush", "Blob", "Shift+B"),
    ("Type", "Type", "T"),
    ("Rectangle", "Rect", "M"),
    ("Ellipse", "Ellipse", "L"),
    ("Eyedropper", "Picker", "I"),
    ("Hand", "Hand", "H"),
    ("Rotate View", "RView", "Shift+H"),
    ("Zoom Tool", "Zoomify", "Z"),
    ("Free Transform", "Transfrm", "E"),
    ("Deselect", "Deselect", "Ctrl+Shift+A"),
    ("Fit Artboard", "Fit", "Ctrl+0"),
    ("100%", "100%", "Ctrl+1"),
    ("Save", "Save", "Ctrl+S"),
    ("Show/Hide Panels", "Panels", "Tab"),
];

/// A parsed shortcut: modifiers and an optional key (none: the modifiers alone, held).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Option<egui::Key>,
}

impl Chord {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Parse `"Ctrl+Shift+Z"`, `"alt"`, `"Space"`, `"]"`. Modifier names: Ctrl/Control/Cmd,
/// Alt/Option, Shift (any case). `Ok(empty)` for an empty string; `Err` names what isn't a key.
pub fn parse(shortcut: &str) -> Result<Chord, String> {
    let mut c = Chord::default();
    let s = shortcut.trim();
    if s.is_empty() {
        return Ok(c);
    }
    // "+" alone or last ("Ctrl++") is the plus key, not a separator.
    let (mods, last) = match s.strip_suffix("++") {
        Some(rest) => (rest, "+"),
        None if s == "+" => ("", "+"),
        None => s.rsplit_once('+').unwrap_or(("", s)),
    };
    for part in mods.split('+').map(str::trim).filter(|p| !p.is_empty()).chain(std::iter::once(last.trim())) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" => c.ctrl = true,
            "alt" | "option" | "opt" => c.alt = true,
            "shift" => c.shift = true,
            _ if c.key.is_none() && part == last.trim() => {
                let name = if part.chars().count() == 1 { part.to_ascii_uppercase() } else { part.to_string() };
                c.key = Some(egui::Key::from_name(&name).or_else(|| egui::Key::from_name(part)).ok_or_else(|| format!("\"{part}\" is not a key"))?);
            }
            _ => return Err(format!("\"{part}\" is not a modifier")),
        }
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Key;

    #[test]
    fn shortcuts_parse() {
        assert_eq!(parse("Ctrl+Z"), Ok(Chord { ctrl: true, key: Some(Key::Z), ..Default::default() }));
        assert_eq!(parse("ctrl + shift + z"), Ok(Chord { ctrl: true, shift: true, key: Some(Key::Z), ..Default::default() }));
        assert_eq!(parse("Alt"), Ok(Chord { alt: true, ..Default::default() }), "a modifier on its own");
        assert_eq!(parse("Space"), Ok(Chord { key: Some(Key::Space), ..Default::default() }));
        assert_eq!(parse("b"), Ok(Chord { key: Some(Key::B), ..Default::default() }));
        assert_eq!(parse("]"), Ok(Chord { key: Some(Key::CloseBracket), ..Default::default() }));
        assert_eq!(parse("Ctrl+0"), Ok(Chord { ctrl: true, key: Some(Key::Num0), ..Default::default() }));
        assert_eq!(parse("Ctrl++"), Ok(Chord { ctrl: true, key: Some(Key::Plus), ..Default::default() }));
        assert_eq!(parse("Tab"), Ok(Chord { key: Some(Key::Tab), ..Default::default() }));
        assert_eq!(parse(""), Ok(Chord::default()));
        assert!(parse("Ctrl+Banana").is_err());
        assert!(parse("Hyper+Z").is_err(), "unknown modifier");
    }

    #[test]
    fn every_preset_parses() {
        for (name, label, s) in PRESETS {
            assert!(parse(s).is_ok_and(|c| !c.is_empty()), "{name}: {s}");
            assert!(label.chars().count() <= 8, "{name}: key label {label:?} is too long for a key");
        }
        for k in PadConfig::default().keys {
            assert!(parse(&k.shortcut).is_ok(), "{k:?}");
        }
    }

    #[test]
    fn the_key_count_grows_with_unused_presets_and_shrinks_from_the_bottom() {
        let mut c = PadConfig::default();
        c.set_key_count(7);
        let shortcuts: Vec<&str> = c.keys.iter().map(|k| k.shortcut.as_str()).collect();
        // The defaults (Undo, Redo, Alt, Space), then the first presets they don't use.
        assert_eq!(shortcuts, ["Ctrl+Z", "Ctrl+Shift+Z", "Alt", "Space", "Shift", "Ctrl", "V"]);
        c.set_key_count(MAX_KEYS + 5);
        assert_eq!(c.keys.len(), MAX_KEYS, "clamped");
        let mut seen: Vec<&str> = c.keys.iter().map(|k| k.shortcut.as_str()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), MAX_KEYS, "no shortcut twice");
        c.set_key_count(3);
        assert_eq!(c.keys.iter().map(|k| k.label.as_str()).collect::<Vec<_>>(), ["Undo", "Redo", "Alt"]);
        c.set_key_count(0);
        assert_eq!(c.keys.len(), MIN_KEYS);
    }

    #[test]
    fn settings_round_trip_and_bad_files_give_defaults() {
        let dir = std::env::temp_dir().join(format!("vectorcraft-pad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pad.json");
        assert_eq!(PadConfig::load(&path), PadConfig::default(), "no file yet");
        let mut c = PadConfig { side: Side::Left, enabled: false, ..Default::default() };
        c.keys[0] = KeySlot::new("Pen", "P");
        c.wheel_modes = vec![WheelMode::History];
        c.save(&path).unwrap();
        assert_eq!(PadConfig::load(&path), c);
        std::fs::write(&path, b"{broken").unwrap();
        assert_eq!(PadConfig::load(&path), PadConfig::default());
        // Missing fields take their defaults; too few keys are filled, repeats and an empty
        // wheel list fixed.
        std::fs::write(&path, br#"{"side":"left","keys":[{"label":"A","shortcut":"A"}],"wheelModes":[]}"#).unwrap();
        let c = PadConfig::load(&path);
        assert_eq!((c.side, c.keys.len(), c.keys[0].label.as_str(), c.enabled), (Side::Left, MIN_KEYS, "A", true));
        // A saved count within the range is kept, an out-of-range one clamped.
        let ten = PadConfig { keys: (0..15).map(|i| KeySlot::new(&i.to_string(), "")).collect(), ..Default::default() }.sanitized();
        assert_eq!(ten.keys.len(), MAX_KEYS);
        let six = PadConfig { keys: (0..6).map(|i| KeySlot::new(&i.to_string(), "")).collect(), ..Default::default() }.sanitized();
        assert_eq!(six.keys.len(), 6);
        assert_eq!(c.wheel_modes, PadConfig::default().wheel_modes);
        let c = PadConfig { wheel_modes: vec![WheelMode::Zoom, WheelMode::Zoom, WheelMode::Rotate], ..Default::default() }.sanitized();
        assert_eq!(c.wheel_modes, [WheelMode::Zoom, WheelMode::Rotate]);
        std::fs::remove_dir_all(dir).ok();
    }
}
