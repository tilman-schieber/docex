// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ai;
mod backend;
mod recovery;
mod snapshot;

use backend::{AppState, Theme, View};
use docex_core::docmodel::DocModel;
use std::time::Duration;
use tauri::{Emitter, State};

// Tauri commands: the work is in `backend`, shared with the dev server.

#[tauri::command]
fn open_doc(state: State<AppState>, path: String) -> Result<View, String> {
    backend::open_doc(&state, path)
}

#[tauri::command]
fn new_doc(state: State<AppState>) -> Result<View, String> {
    backend::new_doc(&state)
}

#[tauri::command]
fn initial_path() -> Option<String> {
    backend::initial_path()
}

#[tauri::command]
fn sync(state: State<AppState>, model: DocModel, tracked: bool) -> Result<View, String> {
    backend::sync(&state, model, tracked)
}

#[tauri::command]
fn resolve(state: State<AppState>, model: DocModel, tracked: bool, wids: Vec<String>, accept: bool) -> Result<View, String> {
    backend::resolve(&state, model, tracked, wids, accept)
}

#[tauri::command]
fn resolve_batch(
    state: State<AppState>,
    model: DocModel,
    tracked: bool,
    accept: Vec<String>,
    reject: Vec<String>,
    done_comments: Vec<String>,
) -> Result<View, String> {
    backend::resolve_batch(&state, model, tracked, accept, reject, done_comments)
}

#[tauri::command]
fn resolve_all(state: State<AppState>, model: DocModel, tracked: bool, accept: bool) -> Result<View, String> {
    backend::resolve_all(&state, model, tracked, accept)
}

#[tauri::command]
fn undo_review(state: State<AppState>, model: DocModel, tracked: bool) -> Result<View, String> {
    backend::undo_review(&state, model, tracked)
}

#[tauri::command]
fn comment_reply(state: State<AppState>, model: DocModel, tracked: bool, id: String, text: String) -> Result<View, String> {
    backend::comment_reply(&state, model, tracked, id, text)
}

#[tauri::command]
fn comment_done(state: State<AppState>, model: DocModel, tracked: bool, id: String, done: bool) -> Result<View, String> {
    backend::comment_done(&state, model, tracked, id, done)
}

#[tauri::command]
fn comment_delete(state: State<AppState>, model: DocModel, tracked: bool, id: String) -> Result<View, String> {
    backend::comment_delete(&state, model, tracked, id)
}

#[tauri::command]
fn set_track(state: State<AppState>, model: DocModel, tracked: bool, on: bool) -> Result<View, String> {
    backend::set_track(&state, model, tracked, on)
}

#[tauri::command]
fn save(state: State<AppState>, model: DocModel, tracked: bool, path: Option<String>) -> Result<View, String> {
    backend::save(&state, model, tracked, path)
}

#[tauri::command]
fn theme() -> Theme {
    backend::theme()
}

#[tauri::command]
fn image_data(path: String) -> Result<String, String> {
    backend::image_data(path)
}

#[tauri::command]
fn snapshot_swap(path: String, paras: Vec<String>) -> Option<snapshot::Snapshot> {
    backend::snapshot_swap(path, paras)
}

#[tauri::command]
fn snapshot_put(path: String, paras: Vec<String>) -> Result<(), String> {
    backend::snapshot_put(path, paras)
}

#[tauri::command]
fn recovery_put(state: State<AppState>, model: DocModel, tracked: bool) -> Result<(), String> {
    backend::recovery_put(&state, model, tracked)
}

#[tauri::command]
fn recovery_check(path: String) -> Option<String> {
    backend::recovery_check(path)
}

#[tauri::command]
fn recovery_open(state: State<AppState>, path: String) -> Result<View, String> {
    backend::recovery_open(&state, path)
}

#[tauri::command]
fn recovery_clear(path: String) {
    backend::recovery_clear(path)
}

#[tauri::command]
fn md_to_model(md: String) -> DocModel {
    backend::md_to_model(md)
}

#[tauri::command]
fn get_author(state: State<AppState>) -> String {
    backend::get_author(&state)
}

#[tauri::command]
fn set_author(state: State<AppState>, name: String) {
    backend::set_author(&state, name)
}

/// Spelling dictionaries Enchant can use ("en_US", "de_DE").
fn dictionaries() -> Vec<String> {
    std::process::Command::new("enchant-lsmod-2")
        .arg("-list-dicts")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.split_whitespace().next().map(String::from)).collect())
        .unwrap_or_default()
}

/// Check spelling in `langs` ("de_DE", "en"; best match per language) as far
/// as dictionaries exist; none turns it off. Returns the languages in use.
#[tauri::command]
fn spellcheck(webview: tauri::Webview, langs: Vec<String>) -> Vec<String> {
    let have = dictionaries();
    let mut use_: Vec<String> = Vec::new();
    for l in &langs {
        let base = l.split('_').next().unwrap_or(l);
        let pick = have.iter().find(|d| *d == l).or_else(|| have.iter().find(|d| d.split('_').next() == Some(base)));
        if let Some(d) = pick {
            if !use_.contains(d) {
                use_.push(d.clone());
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let langs = use_.clone();
        let _ = webview.with_webview(move |wv| {
            use webkit2gtk::{WebContextExt, WebViewExt};
            if let Some(cx) = wv.inner().context() {
                cx.set_spell_checking_enabled(!langs.is_empty());
                let refs: Vec<&str> = langs.iter().map(String::as_str).collect();
                cx.set_spell_checking_languages(&refs);
            }
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = webview;
    use_
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
                let mut last = backend::theme_stamp();
                loop {
                    std::thread::sleep(Duration::from_millis(800));
                    let now = backend::theme_stamp();
                    if now != last {
                        last = now;
                        // Let `omarchy theme set` finish writing.
                        std::thread::sleep(Duration::from_millis(300));
                        let _ = handle.emit("theme-changed", backend::read_theme());
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
            undo_review,
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
            recovery_put,
            recovery_check,
            recovery_open,
            recovery_clear,
            spellcheck,
            get_author,
            set_author
        ])
        .run(tauri::generate_context!())
        .expect("error while running docex");
}
