//! VectorCraft on Android tablets.
//!
//! Runs the same [`vectorcraft_ui_egui::VectorcraftApp`] as the desktop and web apps through
//! eframe's native runner on winit's GameActivity backend (wgpu: Vulkan, or GLES where Vulkan
//! is missing). The Rust library is `libvectorcraft_android.so`; the Gradle project in
//! `android/` packages it with a small Activity (`VectorCraftActivity.java`) that adds what
//! winit doesn't offer on Android: document pickers, documents sent from other apps, and the
//! pen's pressure and tilt (see `vectorcraft-tablet`).
//!
//! Android-friendly controls from the photocraft-android shell, adapted here:
//! - the **Shortcut Pad** ([`pad`]): ExpressKeys-style shortcuts and a turning wheel (zoom,
//!   Rotate View, freehand size, undo/redo) on the window's edge, with its touch router
//!   ([`pad::router`]) owning every finger for the whole window;
//! - **Touch Input** ([`touch`]): two fingers pan, pinch to zoom and twist to rotate the view,
//!   two- and three-finger taps undo and redo, palm rejection while the pen is near, and two
//!   fingers scroll the interface. The pad's gear key opens both settings.
//!
//! Differences from the desktop app:
//! - files are opened and saved through Android's document pickers ([`bridge`], the Storage
//!   Access Framework): the app works on private copies of the picked documents, which are
//!   written back when it saves;
//! - no TCP control server, MCP, native clipboard images, screen color picking, macOS menus
//!   or the browser opener (preferences are held by the app like on the desktop, though);
//! - pen pressure, tilt and the eraser end come from `MotionEvent`s (`vectorcraft-tablet`).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bridge;
pub mod ime;
pub mod keys;
pub mod pad;
pub mod pen_path;
pub mod services;
pub mod touch;

#[cfg(target_os = "android")]
mod app;
