# VectorCraft for Android tablets (prototype)

The same VectorCraft app (engine, PDF and SVG I/O, egui UI) running on Android tablets: an
arm64-v8a APK, minimum Android 8.0 (API 26), built for Android 15 (API 35).

Status: **early prototype.** It builds, installs and starts; document pickers, Save,
Open with/Share and pen input are wired up. Android-friendly touch controls are in (the
Shortcut Pad with its keys and wheel, pinch/rotate/pan touch gestures, palm rejection); they
are listed under [Touch controls](#touch-controls) and being verified on devices.

## How it fits together

```text
VectorCraftActivity.java (extends GameActivity)      libvectorcraft_android.so (this crate)
 ├─ dispatchTouchEvent / dispatchGenericMotionEvent ──JNI──▶ vectorcraft-tablet::android (pen)
 │    (pressure, AXIS_TILT, AXIS_ORIENTATION, tool type)
 ├─ Storage Access Framework pickers   ◀──files──▶ bridge.rs (request.json / answer.json / sync/)
 ├─ Open with / Share  ──────────────────files──▶  bridge/incoming ─▶ app inbox
 └─ GameActivity ──▶ android-activity ──▶ winit ──▶ eframe (wgpu: Vulkan, GLES fallback)
                                                     └─▶ VectorcraftApp (vectorcraft-ui-egui, unchanged)
```

- **Rendering:** eframe's native runner on winit's GameActivity backend; wgpu uses Vulkan, or
  OpenGL ES 3 where Vulkan is missing.
- **Documents:** Android's document pickers (the Storage Access Framework) stand in for the
  desktop's file dialogs: the picked documents are copied into the app's private storage, the
  app works on those copies, and what it writes is copied back to the picked document (the
  [bridge](../src/bridge.rs)) — so Save As, Save, and saved copies of documents sent by other
  apps all persist where the user picked them.
- **Preferences and libraries** live in the app's private storage as on the desktop
  (preferences, Swatches, Graphic Styles, Libraries, Fonts, Data Recovery).
- **Pen:** pressure, tilt and tool type come from `MotionEvent`s via `vectorcraft-tablet`.
- **Touch controls** (below): the Shortcut Pad (`pad/`) and Touch Input gestures (`touch/`), ported
  from the photocraft-android shell's `pad/` and `touch/` modules, adapted to VectorCraft's views
  and tools (the geometry is VectorCraft's turn: pinned-zoom about the fingers, an angle in
  degrees, and the wheel drives `view`/freehand size instead of a raster brush).
- What the desktop has and this doesn't yet: the control server/MCP, the system clipboard with
  its formats, screen-color picking, macOS specifics and the web-only paths.

## Touch controls

- **On the canvas, one finger** draws like the pen (or, in the Touch Input settings, moves the
  canvas or does nothing).
- **Two fingers on the canvas** pan, pinch–zoom and twist–rotate the view, all at once — the art
  stays under the fingers (pinch zooms around them, clamped to the app's own zoom range,
  3.13%–640%; rotation is normalized like the Rotate View tool's). Fingers zoom only once a
  gesture is clearly under way: small jitters don't move the art.
- **Taps** run shortcuts chosen in the settings: a quick two-finger tap undoes (`Ctrl+Z`), a
  three-finger tap redoes (`Ctrl+Shift+Z`) by default — any command's shortcut can be set.
- **Palm rejection:** fingers are ignored while the pen is touching or near the screen; a pen
  touching down drops fingers already resting.
- **A finger held still** on a button, tab or row right-clicks it (context menus, closing
  panels) — on in the Touch Input settings, off there too.
- **The pen's buttons and eraser end** work like a Wacom pen's: the lower side button
  right-clicks (a press while hovering opens the context menu under the pen; held while
  touching, clicks and drags with the right button), the upper side button and the eraser end
  erase with the Eraser tool (back to the previous tool after), and any of them can instead run
  a shortcut or do nothing — pen buttons 1–3 and the eraser end are in the Touch Input settings.
  A pressed button or the eraser end lights up green there, so buttons can be told apart, and
  the button-state figure shows what an unusual pen sends.
- **Menus, panels and lists:** two fingers scroll (and keep gliding after they lift); one finger
  flicking up or down on a draggable list row (Layers panel rows, preset cards) scrolls the list
  instead of dragging; holding a row still grabs it, and a sideways drag moves a slider.
- **The Shortcut Pad** sits on the right edge (a setting moves it left):
  - 2–10 keys (default: Undo, Redo, hold-Alt, hold-Space for pan). A key presses its shortcut
    while touched — `Ctrl+Z` undoes; a modifier alone (`Alt`) is held, so the pen Alt-clicks
    while a finger rests on the key. A key can have a colour of its own (nine presets plus
    anything the picker gives), lightening while it is held.
  - **The wheel** is a touch ring like a pen tablet's Touch Ring: turning clockwise zooms in,
    rotates the view clockwise, grows the freehand tools' size (Pencil, Paintbrush, Blob Brush,
    Eraser — the one `size` option, clamped 0.1–1000 pt like the tool's own), or steps
    undo/redo; counter-clockwise the opposite. Pressing the wheel opens a large ring (its size
    is a setting) that is easier to hit and smoother to turn; tapping the wheel's centre
    switches between the configured functions (Zoom, Rotate, Size, Undo/Redo — any subset, in
    order); the readout (zoom %, rotation °, size pt, or Undo/Redo) shows in the centre while
    it is open.
  - **The screen key** above the keyboard key steps through full screen: normal → full screen
    with the menu bar kept (the touch router keeps working) → full screen, drawn with nothing
    else, no menu bar → back to normal. Drawn like a video player's, showing the next step.
  - **The keyboard key** at the very bottom opens the on-screen keyboard (and closes it again),
    whose letters reach the app's shortcuts.
  - **The gear key** at the very top opens the settings window (note: VectorCraft's `Services`
    has no platform menu to hang menu rows on, so the settings live here rather than in a
    Window › Shortcut Pad menu): the pad's side, key count, every key's label, colour and
    shortcut, the wheel's functions, the large ring's size and its sensitivity (1×: a quarter
    turn doubles the zoom), plus the Touch Input settings (one-finger mode, the gestures and
    taps, palm rejection, hold-to-right-click, and the pen buttons). Both save to the app's
    files directory like the desktop's preferences.


## Building

Prerequisites: JDK 17, the Android SDK (platform 35, build-tools), the Android NDK r27+,
`rustup target add aarch64-linux-android`, `cargo install cargo-ndk`. Then:

```sh
apps/vectorcraft-android/build.sh                 # release APK
PROFILE=dev apps/vectorcraft-android/build.sh     # unoptimised library, faster to build
RELEASE_BUILD=3 apps/vectorcraft-android/build.sh # the 3rd Android release of this version
```

The APK lands in `target/android/vectorcraft-<version>[-<build>]-arm64-v8a.apk`; the version is
the workspace's (`Cargo.toml`), and `RELEASE_BUILD` numbering (0.13.1 build 3 is "0.13.1-3") is
only yours to pick when releasing more than once per version.

Optional: `CRAFT_FONTS_DIR=<craft-fonts checkout>` builds the app with the shared font catalog.

### Installing it

```sh
adb install -r target/android/vectorcraft-*-arm64-v8a.apk
adb logcat -s VectorCraft          # the app's log (see also its Bug reports in Help)
```

## The [`bridge`](../src/bridge.rs)

The Java side (see `VectorCraftActivity.java`) and the Rust side talk through small JSON files
in the app's private storage (`<files>/bridge`), so the whole app stays free of JNI calls (and
of `unsafe` code beyond the one `android_main`):

- `request.json`: Rust asks for a picker; the Activity polls for it (every 150 ms), deletes it,
  and shows the picker.
- `answer.json`: the Activity's answer for that id — copies of the picked documents in
  `inbox/<id>/`, or a path in `out/<id>/` to write to (Save), or `cancelled`.
- `sync/*.json`: Rust wrote a file under the bridge (`{"path"}`); the Activity copies it back to
  the document it stands for, which it remembers by path (Save on an opened document).
- `quit`: the app is leaving (File › Exit, once unsaved work is settled): the Activity copies
  back still-pending saves, finishes, and ends the process (winit's event loop can't be created
  twice in one process).
- `incoming/`: documents another app sent (Open with / Share), copied in by the Activity and
  opened by the app.
- `keyboard-hidden`: left by the Activity when the on-screen keyboard is put away.
