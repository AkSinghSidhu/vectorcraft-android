//! On-screen keyboard text → egui events.
//!
//! winit 0.30 opens the soft keyboard for a focused text field but drops GameActivity's text
//! events, so typing never reached the app. android-activity still keeps the keyboard's text
//! state (`AndroidApp::text_input_state`); the shell polls it every frame while a text field
//! has focus and turns each change into the edits egui understands: Backspace presses for what
//! the keyboard took back (autocorrect, composing a word) and a `Text` event for what it added.
//! The keyboard's buffer is cleared whenever a text field gains focus, so it only ever holds what
//! was typed into the current field since then.

/// What changed between two states of the keyboard's text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    /// Characters to delete before the cursor.
    pub backspaces: usize,
    /// Text to insert after them.
    pub insert: String,
    /// The keyboard's Enter/action key was pressed (it arrives as a newline).
    pub enter: bool,
}

/// The edit that turns `prev` into `now` (the keyboard edits at its end).
pub fn diff(prev: &str, now: &str) -> Edit {
    let common = prev.chars().zip(now.chars()).take_while(|(a, b)| a == b).count();
    let backspaces = prev.chars().count() - common;
    let added: String = now.chars().skip(common).collect();
    let enter = added.contains('\n');
    let insert = added.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    Edit { backspaces, insert, enter }
}

/// The egui events for an edit.
pub fn events(edit: &Edit) -> Vec<egui::Event> {
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let mut out: Vec<egui::Event> = std::iter::repeat_with(|| key(egui::Key::Backspace)).take(edit.backspaces).collect();
    if !edit.insert.is_empty() {
        out.push(egui::Event::Text(edit.insert.clone()));
    }
    if edit.enter {
        out.push(key(egui::Key::Enter));
    }
    out
}

/// The egui events for an edit typed with no text field focused (the Shortcut Pad's keyboard
/// key opened the keyboard): each character is also pressed as its key, so the on-screen
/// keyboard's letters reach the app's shortcuts ("B" picks the brush; with the pad's Ctrl key
/// held, "S" saves).
pub fn key_events(edit: &Edit) -> Vec<egui::Event> {
    let tap = |key: egui::Key| {
        [true, false].map(|pressed| egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: egui::Modifiers::NONE })
    };
    let mut out: Vec<egui::Event> = std::iter::repeat_n(egui::Key::Backspace, edit.backspaces).flat_map(tap).collect();
    for c in edit.insert.chars() {
        if let Some(key) = egui::Key::from_name(&c.to_uppercase().to_string()) {
            out.extend(tap(key));
        }
    }
    if !edit.insert.is_empty() {
        out.push(egui::Event::Text(edit.insert.clone()));
    }
    if edit.enter {
        out.extend(tap(egui::Key::Enter));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfocused_typing_presses_keys() {
        let ev = key_events(&diff("", "b1"));
        let keys: Vec<(egui::Key, bool)> = ev
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed, .. } => Some((*key, *pressed)),
                _ => None,
            })
            .collect();
        assert_eq!(keys, vec![(egui::Key::B, true), (egui::Key::B, false), (egui::Key::Num1, true), (egui::Key::Num1, false)]);
        assert!(matches!(ev.last(), Some(egui::Event::Text(t)) if t == "b1"));
    }

    #[test]
    fn typing_appends() {
        assert_eq!(diff("", "c"), Edit { insert: "c".into(), ..Default::default() });
        assert_eq!(diff("ca", "cat"), Edit { insert: "t".into(), ..Default::default() });
        assert_eq!(diff("cat", "cat"), Edit::default(), "no change, no events");
    }

    #[test]
    fn corrections_delete_then_insert() {
        // Autocorrect replaces the last word: "teh" → "the ".
        assert_eq!(diff("teh", "the "), Edit { backspaces: 2, insert: "he ".into(), enter: false });
        // The keyboard's own backspace while composing.
        assert_eq!(diff("cats", "cat"), Edit { backspaces: 1, ..Default::default() });
        // Characters, not bytes.
        assert_eq!(diff("café", "caf"), Edit { backspaces: 1, ..Default::default() });
        assert_eq!(diff("ü", "üñ"), Edit { insert: "ñ".into(), ..Default::default() });
    }

    #[test]
    fn the_action_key_is_enter() {
        assert_eq!(diff("ok", "ok\n"), Edit { enter: true, ..Default::default() });
        let ev = events(&diff("a", "b\n"));
        assert!(
            matches!(ev.as_slice(), [egui::Event::Key { key: egui::Key::Backspace, .. }, egui::Event::Text(t), egui::Event::Key { key: egui::Key::Enter, .. }] if t == "b")
        );
    }
}
