//! Crash recovery: a copy of the open document with its unsaved edits, kept
//! outside the document while there are unsaved changes, so they survive a
//! crash. Saving (or choosing to discard) removes it.

use crate::snapshot::{abs, data_dir, key};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize)]
struct Meta {
    path: String,
    /// RFC 3339, for showing.
    saved: String,
    /// Seconds since 1970, to compare with the file's own time.
    secs: u64,
}

fn files(path: &str) -> Option<(PathBuf, PathBuf)> {
    let d = data_dir()?.join("recovery");
    let k = key(&abs(path));
    let stem = k.trim_end_matches(".json");
    Some((d.join(format!("{stem}.docx")), d.join(format!("{stem}.json"))))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn put(path: &str, bytes: &[u8], saved: String) -> Result<(), String> {
    let (doc, meta) = files(path).ok_or("no data directory")?;
    std::fs::create_dir_all(doc.parent().unwrap()).map_err(|e| e.to_string())?;
    // Write the copy first: a half-written meta file just means no recovery.
    std::fs::write(&doc, bytes).map_err(|e| e.to_string())?;
    let m = Meta { path: abs(path), saved, secs: now_secs() };
    std::fs::write(meta, serde_json::to_string(&m).unwrap()).map_err(|e| e.to_string())
}

/// The recovery copy of `path` and when it was written, if there is one newer
/// than the file (an older one was overtaken by a save elsewhere and goes).
pub fn get(path: &str) -> Option<(Vec<u8>, String)> {
    let (doc, meta) = files(path)?;
    let m: Meta = serde_json::from_str(&std::fs::read_to_string(&meta).ok()?).ok()?;
    let file_secs = std::fs::metadata(path).and_then(|x| x.modified()).ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    if file_secs > m.secs {
        clear(path);
        return None;
    }
    Some((std::fs::read(doc).ok()?, m.saved))
}

pub fn clear(path: &str) {
    if let Some((doc, meta)) = files(path) {
        let _ = std::fs::remove_file(meta);
        let _ = std::fs::remove_file(doc);
    }
}
