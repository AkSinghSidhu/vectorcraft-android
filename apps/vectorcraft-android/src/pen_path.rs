//! The pen's whole path. A digitizer reports 200 or more positions a second and Android hands
//! them over in batches, one `MotionEvent` with its history per batch; winit turns each event
//! into one touch move at its latest position, so a quick curve reached the brush as a few
//! dozen straight pieces (a large circle came out slightly polygonal). The Activity's pen hook
//! records every sample's position (`vectorcraft_tablet::android::take_pen_path`), and
//! [`splice`] puts them back as touch moves, in order, before the touch router turns them into
//! pointer moves. Each move then also gets its own pen sample's pressure and tilt (the stylus
//! feed shares a frame's samples out over its moves).

use egui::{Event, TouchId, TouchPhase, pos2};

/// How far (points) the samples' latest position may be from winit's latest move.
pub const MAX_DRIFT: f32 = 24.0;

/// Replace the pen's touch moves in `events` with one move per sample of `path` (window pixels,
/// `ppp` pixels per point), placed where its first move was, or just before it lifts when the
/// frame has only the lift. Returns how many moves were put in (0: `events` unchanged, also
/// when no pen touch is in this frame, so the samples wait for a frame that has one).
///
/// A safety net: when the samples' latest position is far from winit's latest move (more than
/// [`MAX_DRIFT`] points), the two don't measure from the same corner, and the moves are kept
/// as winit made them.
pub fn splice(events: &mut Vec<Event>, path: &[(f32, f32)], ppp: f32, is_pen: impl Fn(TouchId) -> bool) -> usize {
    if path.is_empty() || !(ppp.is_finite() && ppp > 0.0) {
        return 0;
    }
    let pen = |e: &Event, phases: &[TouchPhase]| matches!(e, Event::Touch { id, phase, .. } if is_pen(*id) && phases.contains(phase));
    let anchor = events
        .iter()
        .position(|e| pen(e, &[TouchPhase::Move]))
        .or_else(|| events.iter().position(|e| pen(e, &[TouchPhase::End, TouchPhase::Cancel])));
    let Some(at) = anchor else { return 0 };
    let Event::Touch { device_id, id, force, .. } = events[at] else { return 0 };
    let last_move = events.iter().rev().find_map(|e| match e {
        Event::Touch { id: t, phase: TouchPhase::Move, pos, .. } if *t == id => Some(*pos),
        _ => None,
    });
    if let (Some(winit), Some(&(x, y))) = (last_move, path.last())
        && winit.distance(pos2(x / ppp, y / ppp)) > MAX_DRIFT
    {
        return 0;
    }
    let moves = path.iter().map(|&(x, y)| Event::Touch { device_id, id, phase: TouchPhase::Move, pos: pos2(x / ppp, y / ppp), force });
    let mut out = Vec::with_capacity(events.len() + path.len());
    for (i, e) in events.drain(..).enumerate() {
        if i == at {
            out.extend(moves.clone());
        }
        // The pen's own moves this frame are among the samples (each event's latest one).
        if !matches!(&e, Event::Touch { id: t, phase: TouchPhase::Move, .. } if *t == id) {
            out.push(e);
        }
    }
    *events = out;
    path.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::TouchDeviceId;

    fn touch(id: u64, phase: TouchPhase, x: f32) -> Event {
        Event::Touch { device_id: TouchDeviceId(1), id: TouchId(id), phase, pos: pos2(x, 0.0), force: Some(0.5) }
    }

    fn xs(events: &[Event]) -> Vec<(u64, TouchPhase, f32)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Touch { id, phase, pos, .. } => Some((id.0, *phase, pos.x)),
                _ => None,
            })
            .collect()
    }

    /// Ids from 100 up are pens.
    fn pen(id: TouchId) -> bool {
        id.0 >= 100
    }

    #[test]
    fn every_sample_becomes_a_move_in_place_of_the_pens_own() {
        let mut events =
            vec![touch(1, TouchPhase::Move, 5.0), touch(100, TouchPhase::Move, 30.0), Event::PointerGone, touch(100, TouchPhase::Move, 60.0)];
        // Pixels at 2 pixels per point: 20, 40, 60 points.
        assert_eq!(splice(&mut events, &[(40.0, 0.0), (80.0, 0.0), (120.0, 0.0)], 2.0, pen), 3);
        assert_eq!(
            xs(&events),
            [(1, TouchPhase::Move, 5.0), (100, TouchPhase::Move, 20.0), (100, TouchPhase::Move, 40.0), (100, TouchPhase::Move, 60.0)]
        );
        assert!(events.contains(&Event::PointerGone), "other events stay");
    }

    #[test]
    fn the_last_samples_come_before_the_lift() {
        let mut events = vec![touch(100, TouchPhase::End, 9.0)];
        splice(&mut events, &[(8.0, 0.0)], 1.0, pen);
        assert_eq!(xs(&events), [(100, TouchPhase::Move, 8.0), (100, TouchPhase::End, 9.0)]);
    }

    #[test]
    fn samples_measured_from_elsewhere_are_not_used() {
        let before = vec![touch(100, TouchPhase::Move, 30.0)];
        let mut events = before.clone();
        assert_eq!(splice(&mut events, &[(29.0, 0.0), (130.0, 0.0)], 1.0, pen), 0);
        assert_eq!(events, before);
    }

    #[test]
    fn without_a_pen_touch_nothing_changes() {
        let before = vec![touch(1, TouchPhase::Move, 5.0), touch(100, TouchPhase::Start, 3.0)];
        let mut events = before.clone();
        assert_eq!(splice(&mut events, &[(1.0, 0.0)], 1.0, pen), 0);
        assert_eq!(events, before, "a frame with only the touch-down keeps the samples for the next");
        assert_eq!(splice(&mut events, &[], 1.0, pen), 0);
    }
}
