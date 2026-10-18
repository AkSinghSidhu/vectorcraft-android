//! Android: the Activity's pen hook.
//!
//! winit's Android backend forwards a pen's pressure as touch force but drops its tilt,
//! orientation and tool type (tip or eraser). The app's Activity (`PhotoCraftActivity.java`)
//! reads them from every `MotionEvent` before the event goes on to GameActivity and winit, and
//! calls the `native` method below for each pointer and historical sample. Only primitives
//! cross JNI: no `JNIEnv` call is made, so nothing here dereferences a pointer.
//!
//! The one `unsafe` in this module is the `#[unsafe(no_mangle)]` attribute itself, which JNI's
//! name-based lookup (`Java_<package>_<class>_<method>`) needs.

#![allow(unsafe_code)]

use crate::motion::{Pointers, RawEvent, handle, tool};
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

/// A pointer went down, from `PhotoCraftActivity.nativePointer` (Java), before the event goes on
/// to winit: its id and `MotionEvent` tool type.
#[unsafe(no_mangle)]
pub extern "system" fn Java_ai_storyteller_vectorcraft_VectorCraftActivity_nativePointer(_env: *mut c_void, _this: *mut c_void, id: i32, tool_type: i32) {
    if let Ok(id) = u64::try_from(id) {
        POINTERS.lock().unwrap_or_else(PoisonError::into_inner).down(id, tool_type);
    }
    saw_pen(tool_type);
}

/// Route the Activity's pen samples to `callback` (replacing an earlier one).
pub fn install(callback: impl Fn(Option<Sample>) + Send + Sync + 'static) -> Result<(), Error> {
    *SINK.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(callback));
    Ok(())
}

/// One pointer of a `MotionEvent`, from `PhotoCraftActivity.nativePen` (Java). Called on the
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
) {
    saw_pen(tool);
    let e = RawEvent { action, tool, buttons, pressure, tilt, orientation };
    if let Update::Set(sample) = handle(&e) {
        let guard = SINK.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sink) = guard.as_ref() {
            deliver(sink.as_ref(), sample);
        }
    }
}
