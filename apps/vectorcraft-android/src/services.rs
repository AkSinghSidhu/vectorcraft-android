//! Android `Services`: file dialogs through the Storage Access Framework
//! ([`crate::bridge`]), file I/O in the app's private storage — documents the app picked are
//! private copies, written back through the Activity — and documents other apps sent arriving
//! in the [`Inbox`]es.
//!
//! Plain file I/O only, so this module builds and is tested on every platform (the paths are
//! the app's private copies).

use crate::bridge::{self, Bridge, Request};
use egui::Context;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use vectorcraft_ui_egui::picks::{PickAnswer, PickRequest};
use vectorcraft_ui_egui::{FilePick, Services};

/// How often a waiting picker looks for the Activity's answer.
const POLL: Duration = Duration::from_millis(150);

/// Why a folder can't be chosen: Android's document pickers hand over single documents, not
/// folders. Logged where a folder was asked for; the app reads the missing answer as "cancelled".
pub const NO_FOLDER_PICKER: &str = "Choosing a folder isn't supported on Android yet. Pick the documents themselves instead.";

pub fn services(bridge: Arc<Bridge>) -> Services {
    let (b_open, b_open_multi, b_save, b_start) = (bridge.clone(), bridge.clone(), bridge.clone(), bridge.clone());
    let _ = BRIDGE.set(bridge);
    Services {
        // Dialogs a command asked for run on a thread of their own: the app waits on the
        // receiver while the UI keeps animating (as the desktop's async dialogs work).
        start_pick: Some(Box::new(move |request| start_pick(&b_start, request))),
        // Shown in line otherwise (asked outside a command): blocks the UI thread until
        // answered, like the desktop's synchronous dialogs on Windows. A folder: unavailable.
        pick_open: Some(Box::new(move |pick| pick_in_line(&b_open, PickRequest::Open(pick.clone())).and_then(|ps| ps.into_iter().next()))),
        pick_open_multi: Some(Box::new(move || pick_in_line(&b_open_multi, PickRequest::OpenMany).unwrap_or_default())),
        pick_save: Some(Box::new(move |pick| pick_in_line(&b_save, PickRequest::Save(pick.clone())).and_then(|ps| ps.into_iter().next()))),
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        exists: Some(Box::new(|p: &str| Path::new(p).exists())),
        write: Some(Box::new(shared_write)),
        // Background Save and Export write off the UI thread, through the same atomic write and
        // copy-back ([`shared_write`] sees the bridge through the shell's static). The desktop's
        // `write_shared` writes without a copy step; here the Activity copies back on its poll.
        write_shared: Some(std::sync::Arc::new(shared_write)),
        // Native saves stream into the file as they are encoded. Streamed writes are then
        // copied back as a whole by the caller (`write` / `write_shared` marks it).
        stream_shared: Some(std::sync::Arc::new(|p: &str, fill: &mut dyn FnMut(&mut dyn std::io::Write) -> std::io::Result<()>| {
            vectorcraft_engine::cmd::fileio::write_file_with(p, fill).map_err(|e| e.to_string())
        })),
        // Help › links: no system opener is wired up on Android yet; the app reports it.
        open_url: Some(Box::new(|url: &str| log::info!("can't open {url} — no browser handoff on Android yet"))),
        // Documents other apps sent (Open with / Share to): they arrive under bridge/incoming.
        inbox: Some(Arc::new(Mutex::default())),
        place_inbox: Some(Arc::new(Mutex::default())),
        ..Default::default()
    }
}

/// The bridge a copy-back marks through, set when the services are built (a background write
/// runs on a thread of the app's, too far from the builder's captures).
static BRIDGE: std::sync::OnceLock<Arc<Bridge>> = std::sync::OnceLock::new();

/// [`Services::write`] and [`Services::write_shared`]: an atomic write (a failed write keeps the
/// old file), then sync to the picked document when the file is bridge-managed.
fn shared_write(path: &str, bytes: &[u8]) -> Result<(), String> {
    let p = Path::new(path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    vectorcraft_engine::cmd::fileio::write_atomic(p, bytes).map_err(|e| e.to_string())?;
    if let Some(bridge) = BRIDGE.get()
        && let Err(e) = bridge.mark_written(p)
    {
        log::warn!("document write-back not queued: {e}");
    }
    Ok(())
}

/// Show the document picker for `request` on the calling thread (it holds the UI until
/// answered, as the desktop's in-line dialogs do) → the picked paths, or none for a cancel.
fn pick_in_line(bridge: &Bridge, request: PickRequest) -> Option<Vec<String>> {
    ask_sync(bridge, request)
}

/// Asking and waiting, in line. A folder is unavailable; other requests go to the Activity.
fn ask_sync(bridge: &Bridge, request: PickRequest) -> Option<Vec<String>> {
    let (id, wire) = wire_request(bridge, &request)?;
    if let Err(e) = bridge.request(&wire) {
        log::error!("document picker: {e}");
        return None;
    }
    wait_for(bridge, id)
}

/// Ask the Activity for a picker without holding the UI: a thread waits for the answer and is
/// handed back as a receiver, the way the desktop's async dialogs work.
pub fn start_pick(bridge: &Arc<Bridge>, request: PickRequest) -> Option<std::sync::mpsc::Receiver<PickAnswer>> {
    let Some((id, wire)) = wire_request(bridge, &request) else {
        // Folders: nothing asked, nothing answered; the app reads that as a cancel.
        log::warn!("{NO_FOLDER_PICKER}");
        return None;
    };
    if let Err(e) = bridge.request(&wire) {
        log::error!("document picker: {e}");
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let bridge = bridge.clone();
    let spawned = std::thread::Builder::new().name("picker".into()).spawn(move || {
        let answer = wait_for(&bridge, id);
        // A cancellation is no answer at all (`PickAnswer` is the paths that were picked).
        let _ = tx.send(answer.unwrap_or_default());
    });
    if let Err(e) = spawned {
        log::error!("document picker: {e}");
        return None;
    }
    Some(rx)
}

/// The SAF request, with its id; `None` when Android can't ask for it (a folder).
fn wire_request(bridge: &Bridge, request: &PickRequest) -> Option<(u64, Request)> {
    let id = bridge.next_id();
    match request {
        // Providers only filter by MIME type, and most of our formats have none they know
        // (.ai, .eps, plug-ins, scripts…): offer everything, the importer decides.
        PickRequest::Open(_) => Some((id, Request::Open { id, multiple: false, mimes: vec!["*/*".into()] })),
        PickRequest::OpenMany => Some((id, Request::Open { id, multiple: true, mimes: vec!["*/*".into()] })),
        PickRequest::Save(pick) => Some((id, Request::Save { id, mime: bridge::mime_for(&name_for(pick)).into(), name: name_for(pick) })),
        PickRequest::Folder => None,
    }
}

/// A save's suggested document name: the app's, else from its first file type (an empty picker
/// name isn't usable — the SAF picker would have nothing to prefill and the user types one).
fn name_for(pick: &FilePick) -> String {
    let fallback = pick.filters.first().and_then(|(_, exts)| exts.first()).map_or_else(|| "Untitled".to_string(), |e| format!("Untitled.{e}"));
    bridge::file_name(&pick.name, &fallback)
}

/// Wait for request `id`'s answer: the picked paths (an Open's copies stand for the documents;
/// a Save's answer names the one file to write to), or none when it was cancelled or failed.
fn wait_for(bridge: &Bridge, id: u64) -> Option<Vec<String>> {
    loop {
        if let Some(answer) = bridge.take_answer(id) {
            if let Some(e) = &answer.error {
                log::error!("document picker: {e}");
                return None;
            }
            return if answer.cancelled {
                None
            } else if answer.paths.is_empty() {
                answer.path.map(|p| vec![p])
            } else {
                Some(answer.paths)
            };
        }
        std::thread::sleep(POLL);
    }
}

/// Files other apps send arrive in the bridge's `incoming/` folder; a background thread moves
/// them into the app's two inboxes and wakes the UI: one opens them (`Open with VectorCraft`),
/// one places them (`Share to VectorCraft` / `File › Place`, [PlaceArrival]s without a drop
/// point, so the place cursor starts wherever the user next works).
pub fn watch_incoming(bridge: Arc<Bridge>, inbox: vectorcraft_ui_egui::Inbox, place_inbox: vectorcraft_ui_egui::place::PlaceInbox, ctx: Context) {
    let spawned = std::thread::Builder::new().name("incoming".into()).spawn(move || {
        loop {
            let files = bridge.take_incoming();
            if !files.is_empty() {
                inbox
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend(files.iter().cloned().map(|(name, bytes)| (name, bytes.clone())));
                place_inbox
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .extend(files.into_iter().map(|(name, bytes)| vectorcraft_ui_egui::place::PlaceArrival { name, bytes, drop: None }));
                ctx.request_repaint();
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    });
    if let Err(e) = spawned {
        log::error!("incoming documents: {e}");
    }
}

/// Answers' shape once read: paths that were picked, none on a cancel.
#[cfg(test)]
fn closed(answer: crate::bridge::Answer) -> Option<Vec<String>> {
    (!answer.cancelled).then_some(answer.paths)
}

/// Where the app keeps its data when Android names no files directory (never, in practice).
pub fn fallback_files_dir() -> PathBuf {
    std::env::temp_dir().join("vectorcraft")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::Answer;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vectorcraft-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn opens_offer_everything_and_saves_name_the_format() {
        let dir = temp("wire");
        let b = Bridge::new(&dir).unwrap();
        let (id, r) = wire_request(&b, &PickRequest::Open(FilePick::named("a.svg"))).unwrap();
        assert!(matches!(r, Request::Open { multiple: false, mimes, .. } if mimes == vec!["*/*".to_string()]));
        let sid = b.next_id();
        let (_, s) = wire_request(&b, &PickRequest::Save(FilePick::named("Cat.png"))).unwrap();
        assert_ne!(id, sid, "ids keep increasing");
        assert!(matches!(s, Request::Save { name, mime, .. } if name == "Cat.png" && mime == "image/png"));
        let (_, s) = wire_request(&b, &PickRequest::Save(FilePick { filters: vec![("PDF", &["pdf"])], ..Default::default() })).unwrap();
        assert!(matches!(s, Request::Save { name, mime, .. } if name == "Untitled.pdf" && mime == "application/pdf"));
        assert!(wire_request(&b, &PickRequest::Folder).is_none(), "no folder picker");
        assert_eq!(name_for(&FilePick { name: "../../etc/Cat.psd".into(), ..Default::default() }), "Cat.psd");
        std::fs::remove_dir_all(dir).ok();
    }

    /// A save asked off the UI thread answers once an "Activity" written answer appears.
    #[test]
    fn start_pick_answers_off_thread() {
        let dir = temp("pick");
        let b = Bridge::new(&dir).unwrap();
        let _id = b.next_id() + 1; // the id `start_pick` will use
        let activity = {
            let root = b.root().to_path_buf();
            std::thread::spawn(move || {
                for _ in 0..300 {
                    if let Ok(text) = std::fs::read_to_string(root.join("request.json")) {
                        let r: Request = serde_json::from_str(&text).unwrap();
                        let Request::Save { id, name, .. } = r else { panic!("a save request") };
                        assert_eq!(name, "Cat.png");
                        std::fs::remove_file(root.join("request.json")).unwrap();
                        std::fs::create_dir_all(root.join("out/5")).unwrap();
                        let answer = serde_json::json!({ "id": id, "path": root.join("out/5/Cat.png").to_string_lossy() });
                        std::fs::write(root.join("answer.json"), answer.to_string()).unwrap();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                panic!("no request");
            })
        };
        let b = Arc::new(b);
        let rx = start_pick(&b, PickRequest::Save(FilePick::named("Cat.png"))).unwrap();
        activity.join().unwrap();
        let answer = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(answer.len(), 1, "the picked path");
        std::fs::remove_dir_all(dir).ok();
    }

    /// An in-line ask gets its answers the same way. A cancel returns no paths; unreadable
    /// answers are logged, not panics.
    #[test]
    fn cancels_and_errors_read_as_no_paths() {
        let dir = temp("cancel");
        let b = Bridge::new(&dir).unwrap();
        std::fs::write(b.root().join("answer.json"), r#"{ "id": 1, "cancelled": true }"#).unwrap();
        let a: Answer = serde_json::from_str(&std::fs::read_to_string(b.root().join("answer.json")).unwrap()).unwrap();
        assert_eq!(closed(a), None);
        std::fs::write(b.root().join("answer.json"), "{not json").unwrap();
        assert_eq!(b.take_answer(1), None, "garbage is no answer, never a panic");
        std::fs::remove_dir_all(dir).ok();
    }
}
