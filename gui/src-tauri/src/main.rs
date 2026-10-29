// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod snapshot;

use base64::Engine;
use docex_core::comments::Comment;
use docex_core::docmodel::DocModel;
use docex_core::revisions::Change;
use docex_core::{now_iso, ApplyOpts, Docx};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tauri::Emitter;

/// The open document. Edits are applied to it in memory and written on save.
struct Session {
    doc: Docx,
    path: Option<PathBuf>,
    dirty: bool,
    backed_up: bool,
}

#[derive(Default)]
struct AppState {
    session: Mutex<Option<Session>>,
    author: Mutex<String>,
}

/// Everything the editor needs to show a document.
#[derive(Serialize)]
struct View {
    model: DocModel,
    changes: Vec<Change>,
    comments: Vec<Comment>,
    /// Word's "Track Changes" setting of the document.
    track: bool,
    /// Embedded images by target (`media/image1.png`) as data URLs.
    images: BTreeMap<String, String>,
    path: Option<String>,
    dirty: bool,
    warnings: Vec<String>,
}

fn mime(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.ends_with(".jpg") || n.ends_with(".jpeg") {
        "image/jpeg"
    } else if n.ends_with(".gif") {
        "image/gif"
    } else if n.ends_with(".svg") {
        "image/svg+xml"
    } else if n.ends_with(".bmp") {
        "image/bmp"
    } else {
        "image/png"
    }
}

fn data_url(name: &str, bytes: &[u8]) -> String {
    format!("data:{};base64,{}", mime(name), base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn view(s: &Session, warnings: Vec<String>) -> Result<View, String> {
    let doc = &s.doc;
    let mut images = BTreeMap::new();
    for r in doc.rels.iter().filter(|r| r.kind.ends_with("/image") && !r.external) {
        let part = doc.rels.resolve(r.target);
        if let Ok(bytes) = doc.pkg.read(&part) {
            images.insert(r.target.to_string(), data_url(r.target, &bytes));
        }
    }
    Ok(View {
        model: doc.model(),
        changes: doc.changes(),
        comments: doc.comment_list(),
        track: doc.track_changes().map_err(err)?,
        images,
        path: s.path.as_ref().map(|p| p.display().to_string()),
        dirty: s.dirty,
        warnings,
    })
}

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

fn author_of(state: &AppState) -> String {
    let a = state.author.lock().unwrap().clone();
    if !a.trim().is_empty() {
        return a;
    }
    if let Ok(a) = std::env::var("DOCEX_AUTHOR") {
        if !a.trim().is_empty() {
            return a;
        }
    }
    if let Ok(o) = std::process::Command::new("git").args(["config", "user.name"]).output() {
        let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    std::env::var("USER").unwrap_or_else(|_| "docex".into())
}

/// Run `f` on the session after applying the editor's model (if any).
fn with_session<T>(
    state: &tauri::State<AppState>,
    model: Option<&DocModel>,
    tracked: bool,
    f: impl FnOnce(&mut Session, &mut Vec<String>) -> Result<T, String>,
) -> Result<View, String> {
    let author = author_of(state);
    let mut guard = state.session.lock().unwrap();
    let s = guard.as_mut().ok_or("no document open")?;
    let mut warnings = Vec::new();
    if let Some(m) = model {
        let opts = ApplyOpts { author, date: now_iso(), tracked, base_dir: None, keep_opaque: false };
        let r = s.doc.apply_model(m, &opts).map_err(err)?;
        warnings.extend(r.warnings.iter().cloned());
        if r.changed() {
            s.dirty = true;
        }
    }
    f(s, &mut warnings)?;
    view(s, warnings)
}

#[tauri::command]
fn open_doc(state: tauri::State<AppState>, path: String) -> Result<View, String> {
    let p = PathBuf::from(&path);
    let doc = Docx::open(&p).map_err(err)?;
    let s = Session { doc, path: Some(p), dirty: false, backed_up: false };
    let v = view(&s, Vec::new())?;
    *state.session.lock().unwrap() = Some(s);
    Ok(v)
}

#[tauri::command]
fn new_doc(state: tauri::State<AppState>) -> Result<View, String> {
    let bytes = docex_core::template::blank(&now_iso()).map_err(err)?;
    let doc = Docx::from_bytes(bytes).map_err(err)?;
    let s = Session { doc, path: None, dirty: false, backed_up: true };
    let v = view(&s, Vec::new())?;
    *state.session.lock().unwrap() = Some(s);
    Ok(v)
}

/// Path given on the command line, if any.
#[tauri::command]
fn initial_path() -> Option<String> {
    std::env::args().skip(1).find(|a| !a.starts_with('-'))
}

#[tauri::command]
fn sync(state: tauri::State<AppState>, model: DocModel, tracked: bool) -> Result<View, String> {
    with_session(&state, Some(&model), tracked, |_, _| Ok(()))
}

#[tauri::command]
fn resolve(state: tauri::State<AppState>, model: DocModel, tracked: bool, wids: Vec<String>, accept: bool) -> Result<View, String> {
    with_session(&state, Some(&model), tracked, |s, _| {
        if s.doc.resolve_wids(&wids, accept).map_err(err)? > 0 {
            s.dirty = true;
        }
        Ok(())
    })
}

/// Triage: accept and reject many changes (by Word id) in one go.
#[tauri::command]
fn resolve_batch(
    state: tauri::State<AppState>,
    model: DocModel,
    tracked: bool,
    accept: Vec<String>,
    reject: Vec<String>,
    done_comments: Vec<String>,
) -> Result<View, String> {
    with_session(&state, Some(&model), tracked, |s, _| {
        let mut n = 0;
        for id in &done_comments {
            s.doc.set_comment_done(id, true).map_err(err)?;
            n += 1;
        }
        if !accept.is_empty() {
            n += s.doc.resolve_wids(&accept, true).map_err(err)?;
        }
        if !reject.is_empty() {
            n += s.doc.resolve_wids(&reject, false).map_err(err)?;
        }
        if n > 0 {
            s.dirty = true;
        }
        Ok(())
    })
}

#[tauri::command]
fn resolve_all(state: tauri::State<AppState>, model: DocModel, tracked: bool, accept: bool) -> Result<View, String> {
    with_session(&state, Some(&model), tracked, |s, _| {
        if s.doc.resolve_changes(&[], None, accept).map_err(err)? > 0 {
            s.dirty = true;
        }
        Ok(())
    })
}

#[tauri::command]
fn comment_reply(state: tauri::State<AppState>, model: DocModel, tracked: bool, id: String, text: String) -> Result<View, String> {
    let author = author_of(&state);
    with_session(&state, Some(&model), tracked, |s, _| {
        s.doc.reply_comment(&id, &text, &author, &now_iso()).map_err(err)?;
        s.dirty = true;
        Ok(())
    })
}

#[tauri::command]
fn comment_done(state: tauri::State<AppState>, model: DocModel, tracked: bool, id: String, done: bool) -> Result<View, String> {
    with_session(&state, Some(&model), tracked, |s, _| {
        s.doc.set_comment_done(&id, done).map_err(err)?;
        s.dirty = true;
        Ok(())
    })
}

#[tauri::command]
fn comment_delete(state: tauri::State<AppState>, model: DocModel, tracked: bool, id: String) -> Result<View, String> {
    with_session(&state, Some(&model), tracked, |s, _| {
        s.doc.delete_comment(&id).map_err(err)?;
        s.dirty = true;
        Ok(())
    })
}

#[tauri::command]
fn set_track(state: tauri::State<AppState>, model: DocModel, tracked: bool, on: bool) -> Result<View, String> {
    with_session(&state, Some(&model), tracked, |s, _| {
        s.doc.set_track_changes(on).map_err(err)?;
        s.dirty = true;
        Ok(())
    })
}

#[tauri::command]
fn save(state: tauri::State<AppState>, model: DocModel, tracked: bool, path: Option<String>) -> Result<View, String> {
    with_session(&state, Some(&model), tracked, |s, _| {
        if let Some(p) = path {
            s.path = Some(PathBuf::from(p));
            s.backed_up = true;
        }
        let dest = s.path.clone().ok_or("choose where to save")?;
        let bytes = s.doc.to_bytes().map_err(err)?;
        if !s.backed_up && dest.exists() {
            let mut b = dest.as_os_str().to_owned();
            b.push("~");
            std::fs::copy(&dest, &b).map_err(err)?;
            s.backed_up = true;
        }
        let tmp = dest.with_extension("docx.docex-tmp");
        std::fs::write(&tmp, bytes).map_err(err)?;
        std::fs::rename(&tmp, &dest).map_err(err)?;
        s.dirty = false;
        Ok(())
    })
}

/// The active Omarchy theme: its palette (`colors.toml`) and monospace font.
#[derive(Serialize, Clone, PartialEq)]
struct Theme {
    name: Option<String>,
    mode: String,
    colors: BTreeMap<String, String>,
    mono: Option<String>,
}

fn omarchy_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state/omarchy/current"))
}

fn read_theme() -> Theme {
    let dir = omarchy_dir();
    let mut colors = BTreeMap::new();
    let mut mode = "dark".to_string();
    if let Some(text) = dir.as_ref().and_then(|d| std::fs::read_to_string(d.join("theme/colors.toml")).ok()) {
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim().trim_matches('"'));
            if k == "mode" {
                mode = v.to_string();
            } else if v.starts_with('#') {
                colors.insert(k.to_string(), v.to_string());
            }
        }
    }
    let name = dir.and_then(|d| std::fs::read_to_string(d.join("theme.name")).ok()).map(|s| s.trim().to_string());
    let mono = std::process::Command::new("fc-match")
        .args(["monospace", "-f", "%{family}"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).split(',').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty());
    Theme { name, mode, colors, mono }
}

fn theme_stamp() -> Option<SystemTime> {
    let d = omarchy_dir()?;
    let a = std::fs::metadata(d.join("theme/colors.toml")).and_then(|m| m.modified()).ok();
    let b = std::fs::metadata(d.join("theme.name")).and_then(|m| m.modified()).ok();
    a.max(b)
}

#[tauri::command]
fn theme() -> Theme {
    read_theme()
}

/// A local image file as a data URL, for showing newly inserted images.
#[tauri::command]
fn image_data(path: String) -> Result<String, String> {
    let bytes = std::fs::read(&path).map_err(err)?;
    Ok(data_url(&path, &bytes))
}

#[tauri::command]
fn ai_settings() -> ai::SettingsView {
    ai::view(&ai::load())
}

#[tauri::command]
fn ai_settings_set(update: ai::SettingsUpdate) -> Result<ai::SettingsView, String> {
    Ok(ai::view(&ai::update(update)?))
}

#[tauri::command]
async fn ai_complete(app: tauri::AppHandle, req: ai::Request) -> Result<String, String> {
    ai::complete(app, req).await
}

/// The previous snapshot for `path` (or for a similar earlier file), then
/// record `paras` as the new one.
#[tauri::command]
fn snapshot_swap(path: String, paras: Vec<String>) -> Option<snapshot::Snapshot> {
    let prev = snapshot::get(&path).or_else(|| snapshot::similar(&path, &paras));
    let _ = snapshot::put(&path, paras, now_iso());
    prev
}

#[tauri::command]
fn snapshot_put(path: String, paras: Vec<String>) -> Result<(), String> {
    snapshot::put(&path, paras, now_iso())
}

/// Markdown (e.g. pasted text) as a document model fragment.
#[tauri::command]
fn md_to_model(md: String) -> DocModel {
    docex_core::docmodel::from_markdown(&md)
}

#[tauri::command]
fn get_author(state: tauri::State<AppState>) -> String {
    author_of(&state)
}

#[tauri::command]
fn set_author(state: tauri::State<AppState>, name: String) {
    *state.author.lock().unwrap() = name;
}

fn main() {
    // WebKitGTK's DMA-BUF renderer crashes on several Wayland compositors
    // (e.g. Hyprland: "Error 71 (Protocol error)").
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            // Follow `omarchy theme set` live: poll the theme files' timestamps.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut last = theme_stamp();
                loop {
                    std::thread::sleep(Duration::from_millis(800));
                    let now = theme_stamp();
                    if now != last {
                        last = now;
                        // Let `omarchy theme set` finish writing.
                        std::thread::sleep(Duration::from_millis(300));
                        let _ = handle.emit("theme-changed", read_theme());
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_doc,
            new_doc,
            initial_path,
            sync,
            resolve,
            resolve_all,
            resolve_batch,
            ai_settings,
            ai_settings_set,
            ai_complete,
            snapshot_swap,
            snapshot_put,
            comment_reply,
            comment_done,
            comment_delete,
            set_track,
            save,
            image_data,
            theme,
            md_to_model,
            get_author,
            set_author
        ])
        .run(tauri::generate_context!())
        .expect("error while running docex");
}
