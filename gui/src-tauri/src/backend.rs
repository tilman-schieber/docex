//! The editor's commands on the open document, independent of Tauri: the
//! desktop app wraps them as Tauri commands, the dev server (`docex-devserver`)
//! serves them over HTTP for browser tests.

use crate::{recovery, snapshot};
use base64::Engine;
use docex_core::comments::Comment;
use docex_core::docmodel::DocModel;
use docex_core::revisions::Change;
use docex_core::{now_iso, ApplyOpts, Docx};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

/// The open document. Edits are applied to it in memory and written on save.
struct Session {
    doc: Docx,
    path: Option<PathBuf>,
    dirty: bool,
    backed_up: bool,
    /// The document before each review decision (accept, reject, resolve,
    /// delete thread), newest last. Any other edit clears it, so undoing never
    /// drops text typed since.
    undo: Vec<Vec<u8>>,
}

/// Review decisions kept for undo.
const UNDO_DEPTH: usize = 20;

#[derive(Default)]
pub struct AppState {
    session: Mutex<Option<Session>>,
    author: Mutex<String>,
}

/// Everything the editor needs to show a document.
#[derive(Serialize)]
pub struct View {
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
    /// The document's language ("de-DE"), for spell checking.
    lang: Option<String>,
    /// Whether the last review decision can be undone.
    can_undo: bool,
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
        lang: doc.language(),
        warnings,
        can_undo: !s.undo.is_empty(),
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
    state: &AppState,
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
            s.undo.clear();
        }
    }
    f(s, &mut warnings)?;
    view(s, warnings)
}

/// Like `with_session`, for a review decision: remember the document first so
/// it can be undone.
fn with_undo(
    state: &AppState,
    model: &DocModel,
    tracked: bool,
    f: impl FnOnce(&mut Session) -> Result<(), String>,
) -> Result<View, String> {
    with_session(state, Some(model), tracked, |s, _| {
        let snap = s.doc.to_bytes().map_err(err)?;
        f(s)?;
        s.undo.push(snap);
        if s.undo.len() > UNDO_DEPTH {
            s.undo.remove(0);
        }
        Ok(())
    })
}

/// Take back the last review decision.
pub fn undo_review(state: &AppState, model: DocModel, tracked: bool) -> Result<View, String> {
    with_session(state, Some(&model), tracked, |s, _| {
        let snap = s.undo.pop().ok_or("nothing to undo (the document was edited since)")?;
        s.doc = Docx::from_bytes(snap).map_err(err)?;
        // Unsaved even if it matches an earlier state: a save may have written the decision.
        s.dirty = true;
        Ok(())
    })
}

pub fn open_doc(state: &AppState, path: String) -> Result<View, String> {
    let p = PathBuf::from(&path);
    let doc = Docx::open(&p).map_err(err)?;
    let s = Session { doc, path: Some(p), dirty: false, backed_up: false, undo: Vec::new() };
    let v = view(&s, Vec::new())?;
    *state.session.lock().unwrap() = Some(s);
    Ok(v)
}

pub fn new_doc(state: &AppState) -> Result<View, String> {
    let bytes = docex_core::template::blank(&now_iso()).map_err(err)?;
    let doc = Docx::from_bytes(bytes).map_err(err)?;
    let s = Session { doc, path: None, dirty: false, backed_up: true, undo: Vec::new() };
    let v = view(&s, Vec::new())?;
    *state.session.lock().unwrap() = Some(s);
    Ok(v)
}

/// Path given on the command line, if any.
pub fn initial_path() -> Option<String> {
    std::env::args().skip(1).find(|a| !a.starts_with('-'))
}

pub fn sync(state: &AppState, model: DocModel, tracked: bool) -> Result<View, String> {
    with_session(state, Some(&model), tracked, |_, _| Ok(()))
}

pub fn resolve(state: &AppState, model: DocModel, tracked: bool, wids: Vec<String>, accept: bool) -> Result<View, String> {
    with_undo(state, &model, tracked, |s| {
        if s.doc.resolve_wids(&wids, accept).map_err(err)? > 0 {
            s.dirty = true;
        }
        Ok(())
    })
}

/// Triage: accept and reject many changes (by Word id) in one go.
pub fn resolve_batch(
    state: &AppState,
    model: DocModel,
    tracked: bool,
    accept: Vec<String>,
    reject: Vec<String>,
    done_comments: Vec<String>,
) -> Result<View, String> {
    with_undo(state, &model, tracked, |s| {
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

pub fn resolve_all(state: &AppState, model: DocModel, tracked: bool, accept: bool) -> Result<View, String> {
    with_undo(state, &model, tracked, |s| {
        if s.doc.resolve_changes(&[], None, accept).map_err(err)? > 0 {
            s.dirty = true;
        }
        Ok(())
    })
}

pub fn comment_reply(state: &AppState, model: DocModel, tracked: bool, id: String, text: String) -> Result<View, String> {
    let author = author_of(state);
    with_session(state, Some(&model), tracked, |s, _| {
        s.doc.reply_comment(&id, &text, &author, &now_iso()).map_err(err)?;
        s.dirty = true;
        s.undo.clear();
        Ok(())
    })
}

pub fn comment_done(state: &AppState, model: DocModel, tracked: bool, id: String, done: bool) -> Result<View, String> {
    with_undo(state, &model, tracked, |s| {
        s.doc.set_comment_done(&id, done).map_err(err)?;
        s.dirty = true;
        Ok(())
    })
}

pub fn comment_delete(state: &AppState, model: DocModel, tracked: bool, id: String) -> Result<View, String> {
    with_undo(state, &model, tracked, |s| {
        s.doc.delete_comment(&id).map_err(err)?;
        s.dirty = true;
        Ok(())
    })
}

pub fn set_track(state: &AppState, model: DocModel, tracked: bool, on: bool) -> Result<View, String> {
    with_session(state, Some(&model), tracked, |s, _| {
        s.doc.set_track_changes(on).map_err(err)?;
        s.dirty = true;
        s.undo.clear();
        Ok(())
    })
}

pub fn save(state: &AppState, model: DocModel, tracked: bool, path: Option<String>) -> Result<View, String> {
    with_session(state, Some(&model), tracked, |s, _| {
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
        recovery::clear(&dest.display().to_string());
        Ok(())
    })
}

/// The active Omarchy theme: its palette (`colors.toml`) and monospace font.
#[derive(Serialize, Clone, PartialEq)]
pub struct Theme {
    name: Option<String>,
    mode: String,
    colors: BTreeMap<String, String>,
    mono: Option<String>,
}

fn omarchy_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state/omarchy/current"))
}

pub fn read_theme() -> Theme {
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

pub fn theme_stamp() -> Option<SystemTime> {
    let d = omarchy_dir()?;
    let a = std::fs::metadata(d.join("theme/colors.toml")).and_then(|m| m.modified()).ok();
    let b = std::fs::metadata(d.join("theme.name")).and_then(|m| m.modified()).ok();
    a.max(b)
}

pub fn theme() -> Theme {
    read_theme()
}

/// A local image file as a data URL, for showing newly inserted images.
pub fn image_data(path: String) -> Result<String, String> {
    let bytes = std::fs::read(&path).map_err(err)?;
    Ok(data_url(&path, &bytes))
}

/// The previous snapshot for `path` (or for a similar earlier file), then
/// record `paras` as the new one.
pub fn snapshot_swap(path: String, paras: Vec<String>) -> Option<snapshot::Snapshot> {
    let prev = snapshot::get(&path).or_else(|| snapshot::similar(&path, &paras));
    let _ = snapshot::put(&path, paras, now_iso());
    prev
}

pub fn snapshot_put(path: String, paras: Vec<String>) -> Result<(), String> {
    snapshot::put(&path, paras, now_iso())
}

/// Markdown (e.g. pasted text) as a document model fragment.
/// Keep a recovery copy of the open document with the editor's unsaved
/// content, made on a copy: the session (and the editor) stay as they are.
pub fn recovery_put(state: &AppState, model: DocModel, tracked: bool) -> Result<(), String> {
    let author = author_of(state);
    let mut guard = state.session.lock().unwrap();
    let s = guard.as_mut().ok_or("no document open")?;
    let Some(path) = s.path.clone() else { return Ok(()) };
    let mut copy = Docx::from_bytes(s.doc.to_bytes().map_err(err)?).map_err(err)?;
    let opts = ApplyOpts { author, date: now_iso(), tracked, base_dir: None, keep_opaque: false };
    copy.apply_model(&model, &opts).map_err(err)?;
    recovery::put(&path.display().to_string(), &copy.to_bytes().map_err(err)?, now_iso())
}

/// When the recovery copy of `path` was written, if there is one to offer.
pub fn recovery_check(path: String) -> Option<String> {
    recovery::get(&path).map(|(_, saved)| saved)
}

/// Open the recovery copy in place of `path` (unsaved, so the file stays as it is until saved).
pub fn recovery_open(state: &AppState, path: String) -> Result<View, String> {
    let (bytes, _) = recovery::get(&path).ok_or("the recovery copy is gone")?;
    let doc = Docx::from_bytes(bytes).map_err(err)?;
    let s = Session { doc, path: Some(PathBuf::from(&path)), dirty: true, backed_up: false, undo: Vec::new() };
    let v = view(&s, Vec::new())?;
    *state.session.lock().unwrap() = Some(s);
    Ok(v)
}

pub fn recovery_clear(path: String) {
    recovery::clear(&path);
}

/// Marks the selection in the model sent with `ai_source`.
const SEL_START: char = '\u{E000}';
const SEL_END: char = '\u{E001}';

/// The document as docex Markdown, with the editor's content applied: what
/// the AI edits. `region` is the part to edit, in UTF-16 units (JavaScript
/// string indices): the paragraphs (or table, or list) holding the selection.
#[derive(Serialize)]
pub struct Source {
    md: String,
    region: Option<(usize, usize)>,
}

/// Where the selection is in `md`: apply the model with the selection marked
/// to a copy, find the marks in its Markdown, widen to whole blocks.
fn selection_region(doc: &mut Docx, md: &str, marked: &DocModel, author: String) -> Option<(usize, usize)> {
    let mut copy = Docx::from_bytes(doc.to_bytes().ok()?).ok()?;
    let opts = ApplyOpts { author, date: now_iso(), tracked: false, base_dir: None, keep_opaque: true };
    copy.apply_model(marked, &opts).ok()?;
    let with = copy.render(docex_core::render::View::Review);
    let (s, e) = (with.find(SEL_START)?, with.find(SEL_END)?);
    // Only if the marks are all that differs.
    if with.chars().filter(|c| *c != SEL_START && *c != SEL_END).collect::<String>() != md {
        return None;
    }
    let len = SEL_START.len_utf8();
    let (s, e) = if s < e { (s, e - len) } else { (s - len, e) };
    let (s, e) = (s.min(e), s.max(e));
    let a = md[..s].rfind("\n\n").map_or(0, |k| k + 2);
    let b = md[e..].find("\n\n").map_or(md.len(), |k| e + k);
    Some((a, b))
}

fn utf16_at(s: &str, byte: usize) -> usize {
    s[..byte].encode_utf16().count()
}

pub fn ai_source(state: &AppState, model: DocModel, tracked: bool, marked: Option<DocModel>) -> Result<Source, String> {
    let author = author_of(state);
    let mut out = None;
    with_session(state, Some(&model), tracked, |s, _| {
        let md = s.doc.render(docex_core::render::View::Review);
        let region = marked.and_then(|m| selection_region(&mut s.doc, &md, &m, author)).map(|(a, b)| (utf16_at(&md, a), utf16_at(&md, b)));
        out = Some(Source { md, region });
        Ok(())
    })?;
    out.ok_or_else(|| "no document open".into())
}

/// Apply Markdown the AI edited, always as tracked changes in your name.
/// Placeholders it dropped are kept. Undoable like a review decision.
pub fn ai_apply(state: &AppState, model: DocModel, tracked: bool, md: String) -> Result<View, String> {
    let author = author_of(state);
    with_session(state, Some(&model), tracked, |s, warnings| {
        let snap = s.doc.to_bytes().map_err(err)?;
        let opts = ApplyOpts { author, date: now_iso(), tracked: true, base_dir: None, keep_opaque: true };
        let r = s.doc.apply(&md, &opts).map_err(err)?;
        warnings.extend(r.warnings.iter().cloned());
        if r.changed() {
            s.dirty = true;
            s.undo.push(snap);
            if s.undo.len() > UNDO_DEPTH {
                s.undo.remove(0);
            }
        }
        Ok(())
    })
}

pub fn md_to_model(md: String) -> DocModel {
    docex_core::docmodel::from_markdown(&md)
}

pub fn get_author(state: &AppState) -> String {
    author_of(state)
}

pub fn set_author(state: &AppState, name: String) {
    *state.author.lock().unwrap() = name;
}
