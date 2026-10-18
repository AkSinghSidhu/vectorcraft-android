<h1 align="center">VectorCraft for Android</h1>

<p align="center">
  <b>Vector illustration on a touchscreen: the VectorCraft engine as a native Android app.</b>
</p>

<p align="center">
  The open-source VectorCraft editor — an Illustrator-style vector app rebuilt in pure Rust —
  running natively on <b>64-bit ARM Android</b>: arm64-v8a, 60 fps, touch-first controls with a
  shortcut pad, stylus support, and the same <code>.vectorcraft</code> documents the desktop app
  writes.
</p>

<p align="center">
  <img alt="Android: 8.0+ (arm64-v8a)" src="https://img.shields.io/badge/Android-8.0%2B%20%C2%B7%20arm64--v8a-3ddc84?logo=android&logoColor=white">
  <img alt="Written in pure Rust" src="https://img.shields.io/badge/pure-Rust-b83a24?logo=rust&logoColor=white">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-555555">
  <img alt="Status: prototype" src="https://img.shields.io/badge/status-prototype-e8573f">
</p>

---

## Install

Download the APK from [Releases](../../releases) (`vectorcraft-<version>-arm64-v8a.apk`) and open it on the device.

- Android 8.0 (API 26) or newer on a 64-bit ARM device.
- Prototype APKs are signed with the **debug key** so they install as-is. For public distribution
  create a release keystore and switch `signingConfig` in
  [`apps/vectorcraft-android/android/app/build.gradle`](apps/vectorcraft-android/android/app/build.gradle).
- The desktop and web versions live upstream: [storytold/vectorcraft](https://github.com/storytold/vectorcraft).

## Controls

Built for fingers and stylus first; the desktop shortcuts still work with a physical keyboard.

| Touch | Result |
| --- | --- |
| One finger on the canvas | Draw/paint with the active tool (or move the canvas, if set in the pad's settings) |
| Two fingers | **Pinch zoom**, **twist rotate** and **pan** — all anchored on the fingers: the art stays under your fingertips, so starting a gesture never jumps the view |
| Stylus / pen | Draws with pen input; palm rejection — a pen down replaces finger input rather than fighting it |
| Shortcut Pad key (bottom) | Toggle the soft keyboard |

### Shortcut Pad

A strip along the right edge (the side is a setting):

- **⚙ gear (top)** opens the settings window: the pad's side and keys, plus the touch options
  (which tool a one-finger touch drives on the canvas).
- **Keys**: `Undo`, `Redo`, `Alt` (hold for pen+modifier), `Pan` (hold = move the canvas the way
  the desktop Space key does).
- **Wheel**: turn the ring to act, tap its centre to switch what it does — the four mode lights
  show which is active. Modes: `Zoom`, `Rotate`, `Freehand` (tool size of the Pencil,
  Paintbrush, Blob Brush and Eraser), `History` (undo/redo steps).

## Building

Needs **Android SDK 35** + **build-tools 35**, **NDK r27 or newer** (development used
r28.2.13676358), **JDK 17**, **Rust stable** with the aarch64-linux-android target and
**cargo-ndk**:

```sh
rustup target add aarch64-linux-android
cargo install cargo-ndk

export ANDROID_HOME=/path/to/android-sdk
export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/<version>

bash apps/vectorcraft-android/build.sh        # → target/android/vectorcraft-<version>-arm64-v8a.apk
```

Variants: `PROFILE=dev` (unoptimised library, faster iteration), `RELEASE_BUILD=3` (the 3rd
Android release of this engine version; the APK version and updater versionCode follow), and
`CRAFT_FONTS_DIR=<checkout>` to bundle that fonts folder into the app.

What it does: builds the Rust library for arm64-v8a into the Gradle project's `jniLibs`
(`cargo ndk` with path remapping that keeps machine paths out of the binary), then packages and
signs the APK with Gradle (`apps/vectorcraft-android/android`).

CI: pushing a tag `android-v*` (or running
[.github/workflows/android-release.yml](.github/workflows/android-release.yml) manually) builds
the APK and attaches it to a GitHub Release.

## How it fits together

- [`crates/`](crates) — the VectorCraft engine, unchanged from upstream (see
  [README upstream](https://github.com/storytold/vectorcraft) for the desktop app's docs).
- [`apps/vectorcraft-android/`](apps/vectorcraft-android) — the Android client: an
  `eframe`-shell Rust crate plus the GameActivity Gradle project. A small Java↔Rust bridge gives
  the app the pieces Android needs (SAF file pickers for open/save, the soft keyboard, app
  lifecycle), and the touch controls live here too:
  - `src/touch/` — multi-touch gestures (pinch zoom / twist rotate / pan anchored on the
    fingers) and the touch settings, plus the pen-vs-finger router.
  - `src/pad/` — the Shortcut Pad: keys, wheel, and their settings, painted straight into the
    UI's right edge.
- [`crates/tablet/`](crates/tablet) — pen/tablet input plumbing for the platforms.

## Credits

- **[storytold/vectorcraft](https://github.com/storytold/vectorcraft)** — VectorCraft, the
  engine and everything under [`crates/`](crates), by the ArtCraft team. This repository is that
  project with an Android client added on top; the engine's licenses (MIT OR Apache-2.0) and
  [`NOTICE`](NOTICE) apply unchanged.
- **[morte2437/photocraft-android](https://github.com/morte2437/photocraft-android)** — the
  PhotoCraft Android port that proved the GameActivity approach for this codebase's UI, and whose
  Shortcut Pad and touch-input architecture we adapted to the VectorCraft APIs (ported, then
  reworked for VectorCraft's view transform and tokens).
