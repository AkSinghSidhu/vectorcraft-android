//! Android: the Activity's pen hook.
//!
//! winit's Android backend forwards a pen's pressure as touch force but drops its tilt,
//! orientation and tool type (tip or eraser). The app's Activity (`VectorCraftActivity.java`)
//! reads them from every `MotionEvent` before the event goes on to GameActivity and winit, and
//! calls the `native` method below for each pointer and historical sample. Only primitives
//! cross JNI: no `JNIEnv` call is made, so nothing here dereferences a pointer.
//!
//! The one `unsafe` in this module is the `#[unsafe(no_mangle)]` attribute itself, which JNI's
//! name-based lookup (`Java_<package>_<class>_<method>`) needs.

#![allow(unsafe_code)]

use crate::motion::{self, PenControl, Pointers, RawEvent, handle_with, tool};
use crate::{Error, Sample, Update, deliver};
use std::ffi::c_void;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

type Sink = Box<dyn Fn(Option<Sample>) + Send + Sync>;

/// Where pen samples go; set by [`install`].
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// The tool of each pointer on the screen (`nativePointer`).
static POINTERS: Mutex<Pointers> = Mutex::new(Pointers::new());
/// When a pen (tip or eraser) last touched or hovered.
static PEN_SEEN: Mutex<Option<Instant>> = Mutex::new(None);
/// The controls that erase while in use, by [`PenControl::index`] ([`set_erasing_controls`]).
static ERASES: Mutex<[bool; 4]> = Mutex::new([false, false, false, true]);
/// The controls in use in the pen's last event, and the presses since [`take_presses`].
static HELD: Mutex<[bool; 4]> = Mutex::new([false; 4]);
static PRESSES: Mutex<Vec<Press>> = Mutex::new(Vec::new());
/// Where the pen touched in each move sample (historical ones included), window pixels, since
/// [`take_pen_path`].
static PATH: Mutex<Vec<(f32, f32)>> = Mutex::new(Vec::new());
/// The most [`PATH`] holds (a second of a fast digitizer, if nothing takes it).
const PATH_MAX: usize = 480;
/// The button state of the pen's last event, as Android reported it ([`last_buttons`]).
static LAST_BUTTONS: Mutex<i32> = Mutex::new(0);

/// A pen control pressed (a side button) or brought to the screen (the eraser end).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    pub control: PenControl,
    /// Where the pen hovered (window pixels), or `None` when it was touching.
    pub hover: Option<(f32, f32)>,
}

/// The buttons held when touch `id` went down (`MotionEvent.getButtonState()`): a mouse's right
/// button, a pen's side button.
pub fn pointer_buttons(id: u64) -> i32 {
    POINTERS.lock().unwrap_or_else(PoisonError::into_inner).buttons(id)
}

/// The tool of touch `id` (`MotionEvent.getToolType()`), if it was reported.
pub fn pointer_tool(id: u64) -> Option<i32> {
    POINTERS.lock().unwrap_or_else(PoisonError::into_inner).tool(id)
}

/// Which pen controls erase while in use, by [`PenControl::index`].
pub fn set_erasing_controls(erases: [bool; 4]) {
    *ERASES.lock().unwrap_or_else(PoisonError::into_inner) = erases;
}

/// The pen controls in use right now (held buttons, the eraser end near the screen).
pub fn controls_held() -> [bool; 4] {
    *HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The pen's button state as Android last reported it (`MotionEvent.getButtonState()`), so the
/// settings can show what an unusual pen sends.
pub fn last_buttons() -> i32 {
    *LAST_BUTTONS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The pen's positions (window pixels, oldest first) for every move sample since the last call,
/// the ones Android batched between events included. winit passes on only each event's
/// latest position, so a quick curve would be drawn from a few straight pieces.
pub fn take_pen_path() -> Vec<(f32, f32)> {
    std::mem::take(&mut *PATH.lock().unwrap_or_else(PoisonError::into_inner))
}

fn track_path(action: i32, tool_type: i32, x: f32, y: f32) {
    if tool_type != tool::STYLUS && tool_type != tool::ERASER {
        return;
    }
    let mut path = PATH.lock().unwrap_or_else(PoisonError::into_inner);
    match action {
        // A new contact: earlier samples belong to the last stroke.
        motion::action::DOWN => path.clear(),
        motion::action::MOVE if x.is_finite() && y.is_finite() => {
            if path.len() >= PATH_MAX {
                path.remove(0);
            }
            path.push((x, y));
        }
        _ => {}
    }
}

/// The presses since the last call, oldest first.
pub fn take_presses() -> Vec<Press> {
    std::mem::take(&mut *PRESSES.lock().unwrap_or_else(PoisonError::into_inner))
}

fn track_controls(action: i32, tool_type: i32, buttons: i32, x: f32, y: f32) {
    if tool_type != tool::STYLUS && tool_type != tool::ERASER {
        return;
    }
    let gone = matches!(action, motion::action::HOVER_EXIT | motion::action::CANCEL);
    let now = if gone { [false; 4] } else { motion::controls(tool_type, buttons) };
    *LAST_BUTTONS.lock().unwrap_or_else(PoisonError::into_inner) = if gone { 0 } else { buttons };
    let hovering = matches!(action, motion::action::HOVER_ENTER | motion::action::HOVER_MOVE | motion::action::BUTTON_PRESS);
    let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
    for control in PenControl::ALL {
        let i = control.index();
        if now[i] && !held[i] {
            let hover = (hovering && x.is_finite() && y.is_finite()).then_some((x, y));
            let mut presses = PRESSES.lock().unwrap_or_else(PoisonError::into_inner);
            if presses.len() < 16 {
                presses.push(Press { control, hover });
            }
        }
    }
    *held = now;
}

/// Whether touch `id` (winit's touch id: Android's pointer id) is a finger rather than a pen,
/// eraser or mouse.
pub fn is_finger(id: u64) -> bool {
    POINTERS.lock().unwrap_or_else(PoisonError::into_inner).is_finger(id)
}

/// Whether a pen touched or hovered within `d` (palm rejection: a hand resting while the pen is
/// near).
pub fn pen_seen_within(d: Duration) -> bool {
    PEN_SEEN.lock().unwrap_or_else(PoisonError::into_inner).is_some_and(|t| t.elapsed() <= d)
}

fn saw_pen(tool_type: i32) {
    if tool_type == tool::STYLUS || tool_type == tool::ERASER {
        *PEN_SEEN.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }
}

/// A pointer went down, from `VectorCraftActivity.nativePointer` (Java), before the event goes on
/// to winit: its id, `MotionEvent` tool type and button state.
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_vectorcraft_VectorCraftActivity_nativePointer(
    _env: *mut c_void,
    _this: *mut c_void,
    id: i32,
    tool_type: i32,
    buttons: i32,
) {
    if let Ok(id) = u64::try_from(id) {
        POINTERS.lock().unwrap_or_else(PoisonError::into_inner).down(id, tool_type, buttons);
    }
    saw_pen(tool_type);
}

/// Route the Activity's pen samples to `callback` (replacing an earlier one).
pub fn install(callback: impl Fn(Option<Sample>) + Send + Sync + 'static) -> Result<(), Error> {
    *SINK.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(callback));
    Ok(())
}

/// One pointer of a `MotionEvent`, from `VectorCraftActivity.nativePen` (Java). Called on the
/// Activity's UI thread. JNI types: `jint` is `i32`, `jfloat` is `f32`.
///
/// Never unwinds: the mapping doesn't panic and [`deliver`] contains the callback.
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_vectorcraft_VectorCraftActivity_nativePen(
    _env: *mut c_void,
    _this: *mut c_void,
    action: i32,
    tool: i32,
    buttons: i32,
    pressure: f32,
    tilt: f32,
    orientation: f32,
    x: f32,
    y: f32,
) {
    saw_pen(tool);
    track_controls(action, tool, buttons, x, y);
    track_path(action, tool, x, y);
    let e = RawEvent { action, tool, buttons, pressure, tilt, orientation };
    if let Update::Set(sample) = handle_with(&e, *ERASES.lock().unwrap_or_else(PoisonError::into_inner)) {
        let guard = SINK.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sink) = guard.as_ref() {
            deliver(sink.as_ref(), sample);
        }
    }
}
