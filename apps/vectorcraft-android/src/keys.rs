//! Hardware keyboard modifiers.
//!
//! winit 0.30's Android backend forwards key presses, Ctrl, Alt and Shift included, but never
//! reports modifier state (`ModifiersChanged`), so egui saw them as never held: Ctrl+S arrived as
//! "S" (which picks a tool) and no shortcut with a modifier worked. [`Held`] rebuilds the state
//! from the modifier keys' own presses, in event order: Android often delivers Ctrl down, S down
//! and Ctrl up within one frame, so a per-frame snapshot of "what is held now" would already
//! have lost the Ctrl.

use egui::{Event, Key, Modifiers};

/// The modifier keys held, left and right tracked apart (releasing one Shift keeps the other).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    ctrl: [bool; 2],
    alt: [bool; 2],
    shift: [bool; 2],
}

impl Held {
    pub fn modifiers(&self) -> Modifiers {
        let ctrl = self.ctrl.iter().any(|d| *d);
        // Ctrl is the command key off macOS.
        Modifiers { alt: self.alt.iter().any(|d| *d), ctrl, shift: self.shift.iter().any(|d| *d), mac_cmd: false, command: ctrl }
    }

    /// Track a key press or release; `true` if it was a modifier key.
    fn key(&mut self, key: Key, pressed: bool) -> bool {
        let slot = match key {
            Key::ControlLeft => &mut self.ctrl[0],
            Key::ControlRight => &mut self.ctrl[1],
            Key::AltLeft => &mut self.alt[0],
            Key::AltRight => &mut self.alt[1],
            Key::ShiftLeft => &mut self.shift[0],
            Key::ShiftRight => &mut self.shift[1],
            _ => return false,
        };
        *slot = pressed;
        true
    }

    /// Put the modifiers on a frame's input, in order: a `ModifiersChanged` (egui's held state)
    /// at the start and after each modifier key, and on every key and pointer event (egui
    /// matches shortcuts against the event's own modifiers). While Ctrl or Alt is held a key
    /// types no text (winit still reports "s" for Ctrl+S, which would land in a focused field).
    pub fn apply(&mut self, input: &mut egui::RawInput) {
        let events = std::mem::take(&mut input.events);
        let mut out = Vec::with_capacity(events.len() + 2);
        out.push(Event::ModifiersChanged(self.modifiers()));
        for mut e in events {
            let m = self.modifiers();
            match &mut e {
                Event::Key { key, pressed, modifiers, .. } => {
                    let changed = self.key(*key, *pressed);
                    *modifiers = if changed { self.modifiers() } else { m };
                    out.push(e);
                    if changed {
                        out.push(Event::ModifiersChanged(self.modifiers()));
                    }
                    continue;
                }
                Event::Text(_) if m.ctrl || m.alt => continue,
                Event::PointerButton { modifiers, .. } | Event::MouseWheel { modifiers, .. } => *modifiers = m,
                // Keys released while another app had focus never report their release here.
                Event::WindowFocused(false) => *self = Self::default(),
                _ => {}
            }
            out.push(e);
        }
        input.events = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: Key, pressed: bool) -> Event {
        Event::Key { key: k, physical_key: Some(k), pressed, repeat: false, modifiers: Modifiers::NONE }
    }

    fn run(held: &mut Held, events: Vec<Event>) -> Vec<Event> {
        let mut input = egui::RawInput { events, ..Default::default() };
        held.apply(&mut input);
        input.events
    }

    /// Ctrl as egui sets it off macOS: both `ctrl` and `command`.
    const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };

    /// What the emulator delivered for Ctrl+N: all four key events in one frame.
    #[test]
    fn ctrl_n_within_one_frame_is_a_shortcut() {
        let mut held = Held::default();
        let out =
            run(&mut held, vec![key(Key::ControlLeft, true), key(Key::N, true), Event::Text("n".into()), key(Key::ControlLeft, false), key(Key::N, false)]);
        let n_down = out.iter().find_map(|e| match e {
            Event::Key { key: Key::N, pressed: true, modifiers, .. } => Some(*modifiers),
            _ => None,
        });
        assert_eq!(n_down, Some(CTRL), "{out:?}");
        assert!(!out.iter().any(|e| matches!(e, Event::Text(_))), "Ctrl+N types nothing");
        // egui's held state follows the keys: Ctrl on before N, off after.
        let states: Vec<Modifiers> = out
            .iter()
            .filter_map(|e| match e {
                Event::ModifiersChanged(m) => Some(*m),
                _ => None,
            })
            .collect();
        assert_eq!(states, vec![Modifiers::NONE, CTRL, Modifiers::NONE]);
        assert_eq!(held.modifiers(), Modifiers::NONE);
    }

    #[test]
    fn a_held_modifier_carries_across_frames() {
        let mut held = Held::default();
        run(&mut held, vec![key(Key::ShiftLeft, true), key(Key::ControlRight, true)]);
        let out = run(&mut held, vec![key(Key::S, true)]);
        assert!(
            matches!(out.as_slice(), [Event::ModifiersChanged(m), Event::Key { modifiers, .. }] if *m == *modifiers && modifiers.command && modifiers.shift)
        );
        // Releasing one Shift of two keeps Shift held.
        run(&mut held, vec![key(Key::ShiftRight, true), key(Key::ShiftLeft, false)]);
        assert!(held.modifiers().shift);
    }

    #[test]
    fn shift_still_types() {
        let mut held = Held::default();
        let out = run(&mut held, vec![key(Key::ShiftLeft, true), key(Key::A, true), Event::Text("A".into())]);
        assert!(out.iter().any(|e| matches!(e, Event::Text(t) if t == "A")), "Shift+A types an A");
    }

    #[test]
    fn alt_click_carries_alt() {
        let mut held = Held::default();
        let click = Event::PointerButton { pos: egui::pos2(1.0, 1.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
        let out = run(&mut held, vec![key(Key::AltLeft, true), click]);
        assert!(matches!(out.last(), Some(Event::PointerButton { modifiers, .. }) if modifiers.alt));
    }

    #[test]
    fn losing_focus_releases_everything() {
        let mut held = Held::default();
        run(&mut held, vec![key(Key::ControlLeft, true), Event::WindowFocused(false)]);
        assert_eq!(held.modifiers(), Modifiers::NONE);
    }
}
