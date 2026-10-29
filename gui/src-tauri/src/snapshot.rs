//! "Since you last saw it": the accepted text of each document as of the
//! last time it was opened or saved in docex, kept outside the document.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Absolute path of the file it was taken from.
    pub path: String,
    pub name: String,
    /// RFC 3339 time.
    pub taken: String,
    pub paras: Vec<String>,
}

fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("docex").join("snapshots"))
}

fn key(path: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in path.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}.json")
}

fn abs(path: &str) -> String {
    std::fs::canonicalize(path).map(|p| p.display().to_string()).unwrap_or_else(|_| path.to_string())
}

pub fn get(path: &str) -> Option<Snapshot> {
    let f = dir()?.join(key(&abs(path)));
    serde_json::from_str(&std::fs::read_to_string(f).ok()?).ok()
}

pub fn put(path: &str, paras: Vec<String>, taken: String) -> Result<(), String> {
    let d = dir().ok_or("no data directory")?;
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let p = abs(path);
    let name = Path::new(&p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let s = Snapshot { path: p.clone(), name, taken, paras };
    std::fs::write(d.join(key(&p)), serde_json::to_string(&s).unwrap()).map_err(|e| e.to_string())
}

/// For a file seen for the first time (e.g. a renamed attachment), the most
/// similar earlier snapshot of another file, if it shares most paragraphs.
pub fn similar(path: &str, paras: &[String]) -> Option<Snapshot> {
    let d = dir()?;
    let me = abs(path);
    let mine: HashSet<&str> = paras.iter().map(|p| p.as_str()).filter(|p| p.len() > 20).collect();
    if mine.len() < 3 {
        return None;
    }
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&d)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut best: Option<(f64, Snapshot)> = None;
    for (_, f) in files.into_iter().take(300) {
        let Some(s) = std::fs::read_to_string(&f).ok().and_then(|t| serde_json::from_str::<Snapshot>(&t).ok()) else { continue };
        if s.path == me {
            continue;
        }
        let theirs: HashSet<&str> = s.paras.iter().map(|p| p.as_str()).filter(|p| p.len() > 20).collect();
        let common = mine.intersection(&theirs).count() as f64;
        let score = common / (mine.len().max(theirs.len()) as f64);
        if score >= 0.5 && best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, s));
        }
    }
    best.map(|(_, s)| s)
}
